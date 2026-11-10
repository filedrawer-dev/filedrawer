#!/usr/bin/env bash
# ENVIRONMENT's screening settings, staging's or production's (design.md, Submission
# and screening: each has its own API key and its own cap), in their own file so
# service.sh never overwrites them. The unit reads this file after <env>.env, so its
# FILEDRAWER_SCREENER replaces staging's pass-all. Reads the API key as the first line
# of stdin, into $API_KEY; DAILY_USD, the day's cap in dollars, comes as a variable.
# Runs as root; safe to rerun, and a rerun replaces the settings. Exits 2 for an
# environment it doesn't know.
set -euo pipefail
case "${ENVIRONMENT-}" in
  staging | production) ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
API_KEY="${API_KEY-}"
DAILY_USD="${DAILY_USD-}"

# systemd reads single-quoted values literally, so the key may hold anything but a
# single quote or a line break.
if [[ "$API_KEY" != sk-ant-* || "$API_KEY" == *"'"* || "$API_KEY" == *$'\n'* ]]; then
  echo "the API key must start with sk-ant- and hold no single quote or line break" >&2
  exit 1
fi
# The server refuses a cap over $1000 (crates/server/src/config.rs) and wouldn't start.
if [[ ! "$DAILY_USD" =~ ^[0-9]+(\.[0-9]+)?$ ]] \
  || ! awk -v usd="$DAILY_USD" 'BEGIN { exit !(usd <= 1000) }'; then
  echo "DAILY_USD must be dollars a day from 0 to 1000, such as 1 or 0.50" >&2
  exit 1
fi

file="$root/etc/filedrawer/$ENVIRONMENT-screening.env"
mkdir -p "$root/etc/filedrawer"
umask 077
cat > "$file" <<CONF
FILEDRAWER_SCREENER=anthropic
FILEDRAWER_ANTHROPIC_API_KEY='${API_KEY}'
FILEDRAWER_SCREENING_DAILY_USD=${DAILY_USD}
CONF
unset API_KEY
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
systemctl try-restart "filedrawer-$ENVIRONMENT.service"

echo "screening ($ENVIRONMENT): done"
