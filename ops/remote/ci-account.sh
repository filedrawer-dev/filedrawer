#!/usr/bin/env bash
# An account CI submits as on staging, and a new API token for it. ROLE says which:
# smoke, for the smoke test (ops/smoke-staging), or screen, for the screen check
# (ops/smoke-staging screen). Runs as root; safe to rerun. ACCOUNT names the account
# an earlier run made: it is kept when staging still has it, and made again when not.
# Each run adds a token, and the wizard stores only the newest.
# Prints "<account ID><TAB><token>" as the only line on stdout, for the wizard to read.
# Exits 2 for another ROLE, and 3 when staging has no binary yet, so the wizard can
# say to deploy first.
set -euo pipefail

case "${ROLE-}" in
  smoke) display_name="Smoke test (CI)"; token_name="smoke test (CI)" ;;
  screen) display_name="Screen check (CI)"; token_name="screen check (CI)" ;;
  *) echo "ROLE is '${ROLE-}'; it takes smoke or screen" >&2; exit 2 ;;
esac

user=filedrawer env=staging other=production
home="/srv/filedrawer/$env"
# The override is for tests, which point it at a fake server root.
root="${FILEDRAWER_ROOT:-}"
bin="$root$home/bin/filedrawer"
database="$root$home/data/filedrawer.db"

if [[ ! -x "$bin" ]]; then
  echo "staging has no binary at $bin yet" >&2
  exit 3
fi

# admin ARGS...: filedrawer admin, as the service's user on its database, in a
# transient unit under the sandbox of its service's unit (service.sh) with a network
# of its own, so nothing the binary starts outlives the call or binds a port Caddy
# sends traffic to (#117). admin-grant.sh holds the same function. Its stdin is
# /dev/null, since this script arrives on bash's. Each command's unit has a name of its
# own, so `journalctl -u filedrawer-admin-<env>-<command>` shows why a call failed:
# --quiet keeps systemd's own failures, such as a timeout or a sandbox the binary
# can't start in, out of stderr, so a failed call names the unit there.
admin() {
  local unit="filedrawer-admin-$env-$1" status=0
  systemd-run --unit="$unit" --wait --pipe --quiet --collect \
    -p User="$user" -p Group="$user" -p Environment=FILEDRAWER_DATABASE="$database" \
    -p RuntimeMaxSec=60 -p PrivateNetwork=yes -p SocketBindDeny=any \
    -p NoNewPrivileges=yes -p ProtectSystem=strict -p ProtectHome=yes \
    -p PrivateTmp=yes -p PrivateDevices=yes -p ReadWritePaths="$home/data" \
    -p InaccessiblePaths="-/srv/filedrawer/$other" -p ProtectKernelTunables=yes \
    -p ProtectKernelModules=yes -p ProtectControlGroups=yes -p RestrictSUIDSGID=yes \
    -p RestrictNamespaces=yes -p LockPersonality=yes -p MemoryDenyWriteExecute=yes \
    -p RestrictAddressFamilies="AF_INET AF_INET6 AF_UNIX" -p CapabilityBoundingSet= \
    -- "$bin" admin "$@" < /dev/null || status=$?
  if (( status != 0 )); then
    echo "filedrawer admin $1 failed (exit $status; journalctl -u $unit says why)" >&2
  fi
  return "$status"
}

new_token() {
  admin create-token --account "$1" --name "$token_name"
}

account="${ACCOUNT:-}"
token=""
if [[ -n "$account" ]]; then
  errors=$(mktemp)
  trap 'rm -f "$errors"' EXIT
  if ! token=$(new_token "$account" 2> "$errors"); then
    # Only a missing account means making a new one; any other error stops here.
    if ! grep -q "no account" "$errors"; then
      cat "$errors" >&2
      exit 1
    fi
    echo "staging has no account $account any more; making a new one" >&2
    token=""
  fi
fi
if [[ -z "$token" ]]; then
  account=$(admin create-account --display-name "$display_name")
  token=$(new_token "$account")
fi
printf '%s\t%s\n' "$account" "$token"
