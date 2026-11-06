#!/usr/bin/env bash
# Names staging's smoke-test account to the server, in its own settings file: that
# account's entries are screened but never rejected, so CI's smoke test can publish
# them (FILEDRAWER_SMOKE_ACCOUNT; production refuses it). SMOKE_ACCOUNT comes as a
# variable. Runs as root; safe to rerun. Exits 2 for an ID that isn't one.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
account="${SMOKE_ACCOUNT-}"
if [[ ! "$account" =~ ^a-[0-9a-z]+$ ]]; then
  echo "'$account' isn't an account ID (a-...)" >&2
  exit 2
fi

file="$root/etc/filedrawer/staging-smoke.env"
umask 027
printf 'FILEDRAWER_SMOKE_ACCOUNT=%s\n' "$account" > "$file"
chown root:filedrawer "$file"
chmod 0640 "$file"
systemctl daemon-reload
systemctl try-restart filedrawer-staging.service

echo "smoke account setting: done"
