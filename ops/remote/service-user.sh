#!/usr/bin/env bash
# The account ENVIRONMENT's server runs as, and its directories: `filedrawer` for
# staging, `filedrawer-prod` for production, so a binary sent with staging's deploy
# key can't read production's data. Each data folder is its user's alone.
# Settings files in /etc/filedrawer are root's alone (0600), whichever script wrote
# them: systemd reads a unit's EnvironmentFile as root, before it drops to the unit's
# user, so no service user needs them. Runs as root; safe to rerun. Exits 2 for an
# environment it doesn't know.
set -euo pipefail

case "${ENVIRONMENT-}" in
  staging) user=filedrawer ;;
  production) user=filedrawer-prod ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
home="/srv/filedrawer/$ENVIRONMENT"

if ! id "$user" >/dev/null 2>&1; then
  useradd --system --home-dir "$home" --shell /usr/sbin/nologin "$user"
fi

# dir OWNER MODE PATH: PATH under the root, made if missing, with OWNER and MODE.
dir() {
  mkdir -p "$root$3"
  chown "$1" "$root$3"
  chmod "$2" "$root$3"
}
dir root:root 0755 /srv/filedrawer
dir root:root 0755 "$home"
dir root:root 0755 "$home/bin"
dir "$user:$user" 0700 "$home/data"
dir root:root 0755 /etc/filedrawer
for file in "$root"/etc/filedrawer/*.env; do
  [[ -e "$file" ]] || continue
  chown root:root "$file"
  chmod 0600 "$file"
done

echo "service user ($ENVIRONMENT): done"
