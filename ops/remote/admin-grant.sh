#!/usr/bin/env bash
# Makes the owner's account on ENVIRONMENT, staging or production, the admin, so its
# browser session reaches the review queue at /admin (design.md, Admin), and gets the
# daily email. ADMIN_ACCOUNT is the ID the account page shows. Runs as root; safe to
# rerun. Exits 2 for an environment or an ID it doesn't know, and 3 when the
# environment has no binary yet, so the wizard can say to deploy first.
set -euo pipefail

case "${ENVIRONMENT-}" in
  staging) user=filedrawer other=production ;;
  production) user=filedrawer-prod other=staging ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
env="$ENVIRONMENT"
home="/srv/filedrawer/$env"
# The override is for tests, which point it at a fake server root.
root="${FILEDRAWER_ROOT:-}"
bin="$root$home/bin/filedrawer"
database="$root$home/data/filedrawer.db"
account="${ADMIN_ACCOUNT-}"

# admin ARGS...: filedrawer admin, as the service's user on its database, in a
# transient unit under the sandbox of its service's unit (service.sh) with a network
# of its own, so nothing the binary starts outlives the call or binds a port Caddy
# sends traffic to (#117). ci-account.sh holds the same function. Its stdin is
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

if [[ ! "$account" =~ ^a-[0-9a-z]+$ ]]; then
  echo "'$account' isn't an account ID; copy it from the account page (a-...)" >&2
  exit 2
fi
if [[ ! -x "$bin" ]]; then
  echo "$ENVIRONMENT has no binary at $bin yet" >&2
  exit 3
fi

admin grant --account "$account"
