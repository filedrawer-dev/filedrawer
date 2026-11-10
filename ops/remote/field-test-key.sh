#!/usr/bin/env bash
# Opens, replaces or closes staging's field-test key (design.md, Hosting): a key in a
# URL's query, `?key=<hex>`, that lets agents' fetch tools, which can't send the
# staging key's header, read staging's pages past Caddy's gate until it expires.
# ops/field-test-key runs it as root on the VPS.
#
# Reads FIELD_TEST, which the remote shell reads from stdin's first line ahead of
# this script, so the key never shows in a command line:
#   open <64 hex> <hours>      refused (exit 3) while an unexpired key is open
#   replace <64 hex> <hours>   the same, over an open key
#   close
# Hours: 1 to 72. The expiry comes from this server's clock and sits in Caddy's
# matcher, so the key lapses without a timer.
#
# Writes /etc/caddy/field-test.caddy, which caddy.sh's gate imports for staging and
# its files host, validates the Caddyfile as the caddy user, reloads Caddy, then
# probes it locally, with the staging key read from staging's site file
# (/etc/caddy/sites/staging.caddy), or from the Caddyfile on a server set up before
# the site files. For open and replace, a failed validate, reload or probe puts
# the previous snippet back (exit 1). For close, only a failed validate or reload
# does, since putting it back would reopen the key: a probe that finds the old key
# still let through exits 1, and one that finds staging down only warns. Prints
# `expires=<UTC time>` or `closed`, never the key. Exit 2: a line it doesn't expect.
set -euo pipefail
: "${FIELD_TEST?set FIELD_TEST to open|replace <key> <hours>, or close}"
line="$FIELD_TEST"
unset FIELD_TEST
# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
dir="$root/etc/caddy"
caddyfile="$dir/Caddyfile"
snippet="$dir/field-test.caddy"
host=staging.filedrawer.dev
stale="field-test key not valid here: it is expired or replaced, or the request isn't a GET or HEAD of a page. Ask the owner to run ops/field-test-key open."

if [[ "$line" =~ ^(open|replace)\ ([0-9a-f]{64})\ ([1-9][0-9]?)$ ]] \
  && (( BASH_REMATCH[3] <= 72 )); then
  verb="${BASH_REMATCH[1]}" key="${BASH_REMATCH[2]}" hours="${BASH_REMATCH[3]}"
elif [[ "$line" == close ]]; then
  verb=close key="" hours=0
else
  echo "field-test-key: expected 'open|replace <64 hex digits> <hours 1-72>' or 'close'" >&2
  exit 2
fi

# The trailing x keeps the snippet's final newlines, which $(...) would drop.
previous=$( { [[ -f "$snippet" ]] && cat "$snippet"; } ; printf x)
previous="${previous%x}"
now=$(date +%s)
# The key the snippet now holds, and its expiry, from the snippet's first line.
old_key="" old_expiry=""
if [[ "$previous" =~ ^#\ field-test\ key=([0-9a-f]{64})\ expires=([0-9]+)$'\n' ]]; then
  old_key="${BASH_REMATCH[1]}" old_expiry="${BASH_REMATCH[2]}"
fi
if [[ $verb == open && -n "$old_key" && -n "$old_expiry" ]] && (( now < old_expiry )); then
  echo "field-test-key: a key is open until $(date -u -d "@$old_expiry" +%FT%TZ); replace it, or close it first" >&2
  exit 3
fi

# put TEXT: makes TEXT the snippet, readable by Caddy and by no one else, in one
# rename.
put() {
  local tmp
  tmp=$(mktemp "$dir/.field-test.XXXXXX")
  printf '%s' "$1" > "$tmp"
  chown root:caddy "$tmp"
  chmod 640 "$tmp"
  mv "$tmp" "$snippet"
}

# fail WHY: puts the previous snippet back and stops.
fail() {
  put "$previous"
  systemctl reload caddy || true
  echo "field-test-key: $1; the previous snippet is back" >&2
  exit 1
}

if [[ $verb == close ]]; then
  text=""
else
  expiry=$(( now + hours * 3600 ))
  text="# field-test key=$key expires=$expiry
@field_test {
	method GET HEAD
	not path /staging* /admin*
	query key=$key
}
handle @field_test {
	@field_test_expired expression int({time.now.unix}) >= $expiry
	handle @field_test_expired {
		respond \"$stale\" 403
	}
	handle {
		uri query -key
		reverse_proxy 127.0.0.1:8081
	}
}
"
fi

put "$text"
runuser -u caddy -- caddy validate --config "$caddyfile" --adapter caddyfile > /dev/null \
  || fail "caddy validate refused it"
systemctl reload caddy || fail "caddy didn't reload"

# status CONFIG: the status Caddy answers a GET of /tags with the curl config lines
# CONFIG, read from stdin, so no key is an argument.
status() {
  printf 'url = "https://%s/tags%s"\n' "$host" "$1" \
    | { if [[ -n "${2:-}" ]]; then cat; printf 'header = "%s"\n' "$2"; else cat; fi; } \
    | curl -K - -s -o /dev/null -w '%{http_code}' --max-time 10 \
        --resolve "$host:443:127.0.0.1" || true
}
# probe_failed WHY: on open or replace, back to the previous snippet; on close, a
# warning, since the old key's own probe is what says whether it closed.
probe_failed() {
  if [[ $verb == close ]]; then
    echo "field-test-key: warning: $1" >&2
  else
    fail "$1"
  fi
}
staging_key=""
for gate in "$dir/sites/staging.caddy" "$caddyfile"; do
  [[ -f "$gate" ]] || continue
  staging_key=$(sed -n 's/.*@agent header X-Staging-Key "\([0-9a-f]\{64\}\)"$/\1/p' "$gate")
  [[ -z "$staging_key" ]] || break
done

got=$(status "")
[[ $got == 401 ]] || probe_failed "a request with no key got $got, not 401"
if [[ -z "$staging_key" ]]; then
  probe_failed "neither staging's site file nor the Caddyfile holds a staging key to probe with"
else
  got=$(status "" "X-Staging-Key: $staging_key")
  [[ $got == 200 || $got == 502 ]] || probe_failed "the staging key got $got"
fi
if [[ $verb != close ]]; then
  got=$(status "?key=$key")
  [[ $got == 200 || $got == 502 ]] || fail "the new key got $got"
fi
if [[ -n "$old_key" && "$old_key" != "$key" ]]; then
  got=$(status "?key=$old_key")
  if [[ $got != 403 ]]; then
    [[ $verb == close ]] || fail "the old key got $got, not 403"
    echo "field-test-key: the old key still got $got, not 403, so it may still be open" >&2
    exit 1
  fi
fi

if [[ $verb == close ]]; then
  echo closed
else
  echo "expires=$(date -u -d "@$expiry" +%FT%TZ)"
fi
