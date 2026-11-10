#!/usr/bin/env bash
# Caddy in front of ENVIRONMENT, staging or production. The Caddyfile imports one site
# file per environment, /etc/caddy/sites/<env>.caddy, and a run writes only its own,
# so provisioning one environment never rewrites the other's sites. The new files go
# in place only if `caddy validate` takes the whole config; otherwise, or if the run
# stops before Caddy's verdict, the ones before are put back and Caddy isn't
# reloaded. A Caddyfile from before the site files holds staging's sites itself, so
# only staging's run replaces it; production's refuses until then.
#
# Production: filedrawer.dev and files.filedrawer.dev, open to all, and
# www.filedrawer.dev redirecting to filedrawer.dev. While the server doesn't answer,
# before the first deploy or during a restart, they answer a plain 503 with
# Retry-After instead of Caddy's bare 502.
#
# Staging: every request but robots.txt needs either basic auth, for the owner's
# browser, or the staging key in an X-Staging-Key header, for agents: an agent's API
# token needs the Authorization header that basic auth uses. While
# ops/field-test-key holds one open, a field-test key in the query (`?key=`) lets
# reads of pages through too, for fetch tools that can't send a header; its snippet,
# /etc/caddy/field-test.caddy, is made here empty only when missing, so a rerun keeps
# an open key.
# The browser's Authorization: Basic header reaches the server too, where only a
# Bearer token marks a request as an agent's (respond::wants_page); ops/smoke-staging
# checks that a browser still gets the pages.
# For staging, reads "<password><TAB><key>" as the first line of stdin, so neither
# appears in a command line that `ps` could show; STAGING_USER names the basic-auth
# user. An empty password keeps the hash the server already holds for that user, in
# staging's site file or, before the split into site files, the Caddyfile, as
# `ops/provision-vps --only-new` sends.
#
# Runs as root on Debian; safe to rerun. Exits 2 for an environment it doesn't know.
set -euo pipefail
case "${ENVIRONMENT-}" in
  staging | production) ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
if [[ $ENVIRONMENT == staging ]]; then
  : "${STAGING_USER:?set STAGING_USER}"
  : "${STAGING_SECRETS:?pass '<password><TAB><key>' as the first line of stdin}"
  STAGING_PASSWORD="${STAGING_SECRETS%%$'\t'*}"
  STAGING_KEY="${STAGING_SECRETS#*$'\t'}"
  unset STAGING_SECRETS
  [[ "$STAGING_KEY" =~ ^[0-9a-f]{64}$ ]] \
    || { echo "expected a 64-hex-digit staging key" >&2; exit 1; }
fi
# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
dir="$root/etc/caddy"
caddyfile="$dir/Caddyfile"
site="$dir/sites/$ENVIRONMENT.caddy"

if ! command -v caddy >/dev/null 2>&1; then
  # Caddy's own apt repository, as its install docs give it.
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    > /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -q
  DEBIAN_FRONTEND=noninteractive apt-get -y -q install caddy
fi

# staging_site: staging's site file, its gate and its two hosts.
staging_site() {
  local hash old
  if [[ -n "$STAGING_PASSWORD" ]]; then
    hash=$(caddy hash-password --plaintext "$STAGING_PASSWORD")
  else
    hash=""
    for old in "$site" "$caddyfile"; do
      [[ -f "$old" ]] || continue
      hash=$(awk -v user="$STAGING_USER" \
        '$1 == user && $2 ~ /^\$2[aby]\$/ { print $2; exit }' "$old")
      [[ -z "$hash" ]] || break
    done
    if [[ -z "$hash" ]]; then
      echo "no password given and no basic-auth hash for $STAGING_USER to keep; run the full ops/provision-vps to set one" >&2
      exit 1
    fi
  fi
  cat <<CONF
# Written by ops/remote/caddy.sh for staging; edits here are lost on the next run.

# Staging is the owner's alone: every request but robots.txt needs the staging key,
# for agents, whose API token needs the Authorization header that basic auth uses,
# or the password, for the owner's browser. No crawler should index it. An open
# field-test key (ops/remote/field-test-key.sh) lets GETs of pages through; any other
# request with a key in its query gets a 403 that says the key isn't valid here.
(staging_gate) {
	handle /robots.txt {
		respond "User-agent: *
Disallow: /
"
	}
	@agent header X-Staging-Key "${STAGING_KEY}"
	handle @agent {
		request_header -X-Staging-Key
		reverse_proxy 127.0.0.1:8081
	}
	import /etc/caddy/field-test.caddy
	@field_test_stale query key=*
	handle @field_test_stale {
		respond "field-test key not valid here: it is expired or replaced, or the request isn't a GET or HEAD of a page. Ask the owner to run ops/field-test-key open." 403
	}
	handle {
		basic_auth {
			${STAGING_USER} ${hash}
		}
		reverse_proxy 127.0.0.1:8081
	}
}

staging.filedrawer.dev {
	import staging_gate
}

# Staging's attached files (design.md, Attached code): the same process, which
# answers only /f/<sha256> on this host.
files.staging.filedrawer.dev {
	import staging_gate
}
CONF
}

# production_site: production's site file.
production_site() {
  cat <<'CONF'
# Written by ops/remote/caddy.sh for production; edits here are lost on the next run.

# While the server doesn't answer, before the first deploy or during a restart, a
# short plain answer instead of Caddy's bare 502.
(production_down) {
	handle_errors {
		header Retry-After 120
		respond "File Drawer isn't answering right now: it is starting up, or not open yet. Try again in a few minutes." 503
	}
}

filedrawer.dev {
	reverse_proxy 127.0.0.1:8080
	import production_down
}

# Attached files (design.md, Attached code): the same process, which answers only
# /f/<sha256> on this host.
files.filedrawer.dev {
	reverse_proxy 127.0.0.1:8080
	import production_down
}

# People type www, though browsers don't add it (design.md, Hosting and operations).
www.filedrawer.dev {
	redir https://filedrawer.dev{uri} permanent
}
CONF
}

import_line="import /etc/caddy/sites/*.caddy"
if [[ $ENVIRONMENT != staging && -f "$caddyfile" ]] \
  && ! grep -qxF "$import_line" "$caddyfile"; then
  echo "the Caddyfile still holds staging's sites from before the site files; run staging's Caddy stage first (ops/provision-vps --only-new), which moves them to their own file" >&2
  exit 1
fi

text=$("${ENVIRONMENT}_site")
unset STAGING_PASSWORD

if [[ $ENVIRONMENT == staging ]]; then
  snippet="$dir/field-test.caddy"
  if [[ ! -e "$snippet" ]]; then
    : > "$snippet"
    chown root:caddy "$snippet"
    chmod 640 "$snippet"
  fi
fi

mkdir -p "$dir/sites"
# The files as they were, to put back if Caddy refuses the new config.
saved=$(mktemp -d)
for file in "$caddyfile" "$site"; do
  [[ -f "$file" ]] && cp -p "$file" "$saved/$(basename "$file")"
done
# put_back: each file as it was, or gone if there was none.
put_back() {
  local file
  for file in "$caddyfile" "$site"; do
    if [[ -f "$saved/$(basename "$file")" ]]; then
      cp -p "$saved/$(basename "$file")" "$file"
    else
      rm -f "$file"
    fi
  done
}

# Until Caddy takes the new config, any exit, a signal's included (ssh sends SIGHUP
# when its session drops), puts the files before back.
accepted=0
on_exit() {
  (( accepted )) || put_back
  rm -rf "$saved"
}
trap on_exit EXIT
trap 'exit 1' HUP INT TERM

# swap_in PATH TEXT: PATH holds TEXT, swapped in whole.
swap_in() {
  local tmp
  tmp=$(mktemp "$(dirname "$1")/.caddy.XXXXXX")
  printf '%s\n' "$2" > "$tmp"
  chmod 0644 "$tmp"
  mv "$tmp" "$1"
}
swap_in "$site" "$text"
swap_in "$caddyfile" "# Written by ops/remote/caddy.sh; edits here are lost on the next run.
# Each environment's sites are in a file of their own, which caddy.sh writes for that
# environment alone.
$import_line"

if ! caddy validate --config "$caddyfile" --adapter caddyfile; then
  echo "caddy validate refused the new config; the files before are back, and Caddy wasn't reloaded" >&2
  exit 1
fi
accepted=1
systemctl enable caddy
systemctl reload-or-restart caddy

echo "caddy ($ENVIRONMENT): done"
