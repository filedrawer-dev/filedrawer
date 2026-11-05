#!/usr/bin/env bash
# Caddy in front of staging, with basic auth on every request but robots.txt.
# Reads the basic-auth password as the first line of stdin, so it never appears in
# a command line that `ps` could show for long; STAGING_USER names the user. Runs as
# root on Debian; safe to rerun.
set -euo pipefail
: "${STAGING_USER:?set STAGING_USER}"
STAGING_PASSWORD="${STAGING_PASSWORD:?pass the password as the first line of stdin}"

if ! command -v caddy >/dev/null 2>&1; then
  # Caddy's own apt repository, as its install docs give it.
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    > /etc/apt/sources.list.d/caddy-stable.list
  apt-get update -q
  DEBIAN_FRONTEND=noninteractive apt-get -y -q install caddy
fi

hash=$(caddy hash-password --plaintext "$STAGING_PASSWORD")
unset STAGING_PASSWORD

cat > /etc/caddy/Caddyfile <<CONF
# Written by ops/remote/caddy.sh; edits here are lost on the next run.

staging.filedrawer.dev {
	# Staging is the owner's alone, and no crawler should index it.
	handle /robots.txt {
		respond "User-agent: *
Disallow: /
"
	}
	handle {
		basic_auth {
			${STAGING_USER} ${hash}
		}
		reverse_proxy 127.0.0.1:8081
	}
}
CONF
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl enable caddy
systemctl reload-or-restart caddy

echo "caddy: done"
