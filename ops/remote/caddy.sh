#!/usr/bin/env bash
# Caddy in front of staging. Every request but robots.txt needs either basic auth,
# for the owner's browser, or the staging key in an X-Staging-Key header, for agents:
# an agent's API token needs the Authorization header that basic auth uses.
# Reads "<password><TAB><key>" as the first line of stdin, so neither appears in a
# command line that `ps` could show; STAGING_USER names the basic-auth user. Runs as
# root on Debian; safe to rerun.
set -euo pipefail
: "${STAGING_USER:?set STAGING_USER}"
: "${STAGING_SECRETS:?pass '<password><TAB><key>' as the first line of stdin}"
STAGING_PASSWORD="${STAGING_SECRETS%%$'\t'*}"
STAGING_KEY="${STAGING_SECRETS#*$'\t'}"
unset STAGING_SECRETS
[[ -n "$STAGING_PASSWORD" && "$STAGING_KEY" =~ ^[0-9a-f]{64}$ ]] \
  || { echo "expected a password and a 64-hex-digit staging key" >&2; exit 1; }

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
	# Agents send the staging key, leaving Authorization for their API token.
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

# Staging's attached files (design.md, Attached code): the same process, which
# answers only /f/<sha256> on this host. Behind the same key and password.
files.staging.filedrawer.dev {
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
CONF
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl enable caddy
systemctl reload-or-restart caddy

echo "caddy: done"
