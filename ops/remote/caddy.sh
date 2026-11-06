#!/usr/bin/env bash
# Caddy in front of staging. Every request but robots.txt needs either basic auth,
# for the owner's browser, or the staging key in an X-Staging-Key header, for agents:
# an agent's API token needs the Authorization header that basic auth uses.
# Reads "<password><TAB><key>" as the first line of stdin, so neither appears in a
# command line that `ps` could show; STAGING_USER names the basic-auth user. An empty
# password keeps the hash the Caddyfile already holds for that user, as
# `ops/provision-vps --only-new` sends. Runs as root on Debian; safe to rerun.
set -euo pipefail
: "${STAGING_USER:?set STAGING_USER}"
: "${STAGING_SECRETS:?pass '<password><TAB><key>' as the first line of stdin}"
STAGING_PASSWORD="${STAGING_SECRETS%%$'\t'*}"
STAGING_KEY="${STAGING_SECRETS#*$'\t'}"
unset STAGING_SECRETS
[[ "$STAGING_KEY" =~ ^[0-9a-f]{64}$ ]] \
  || { echo "expected a 64-hex-digit staging key" >&2; exit 1; }
# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
caddyfile="$root/etc/caddy/Caddyfile"

if ! command -v caddy >/dev/null 2>&1; then
  # Caddy's own apt repository, as its install docs give it.
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    > /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -q
  DEBIAN_FRONTEND=noninteractive apt-get -y -q install caddy
fi

if [[ -n "$STAGING_PASSWORD" ]]; then
  hash=$(caddy hash-password --plaintext "$STAGING_PASSWORD")
else
  hash=$( [[ -f "$caddyfile" ]] && awk -v user="$STAGING_USER" \
    '$1 == user && $2 ~ /^\$2[aby]\$/ { print $2; exit }' "$caddyfile" || true)
  if [[ -z "$hash" ]]; then
    echo "no password given and no basic-auth hash for $STAGING_USER to keep; run the full ops/provision-vps to set one" >&2
    exit 1
  fi
fi
unset STAGING_PASSWORD

cat > "$caddyfile" <<CONF
# Written by ops/remote/caddy.sh; edits here are lost on the next run.

# Staging is the owner's alone: every request but robots.txt needs the staging key,
# for agents, whose API token needs the Authorization header that basic auth uses,
# or the password, for the owner's browser. No crawler should index it.
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
caddy validate --config "$caddyfile" --adapter caddyfile
systemctl enable caddy
systemctl reload-or-restart caddy

echo "caddy: done"
