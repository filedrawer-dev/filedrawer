#!/usr/bin/env bash
# Production's healthchecks.io ping URLs (design.md, Hosting and operations): the daily
# email's, which the server reads as FILEDRAWER_DAILY_EMAIL_PING and without which
# production refuses to start, and the nightly copy's (#41), which its timer reads as
# BACKUP_PING from a file of its own. Reads "<daily URL><TAB><backup URL>" as the
# first line of stdin, into $PINGS: anyone holding a ping URL can report a run, so
# neither sits in a command line. Each must be an https URL on hc-ping.com with a
# path, as the server checks. Runs as root; safe to rerun, and a rerun replaces both.
set -euo pipefail
: "${PINGS?pass '<daily URL><TAB><backup URL>' as the first line of stdin}"

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
ping_url='^https://hc-ping\.com/[A-Za-z0-9_./-]+$'
if [[ "$PINGS" != *$'\t'* ]]; then
  echo "expected '<daily URL><TAB><backup URL>'" >&2
  exit 1
fi
daily="${PINGS%%$'\t'*}"
backup="${PINGS#*$'\t'}"
unset PINGS
# The URLs are secrets, so a refusal names which one, never its text.
for which in daily backup; do
  url="$daily"
  [[ $which == daily ]] || url="$backup"
  if [[ ! "$url" =~ $ping_url ]]; then
    echo "the $which ping isn't a healthchecks.io ping URL, such as https://hc-ping.com/<uuid>" >&2
    exit 1
  fi
done

dir="$root/etc/filedrawer"
mkdir -p "$dir"
umask 077
printf 'FILEDRAWER_DAILY_EMAIL_PING=%s\n' "$daily" > "$dir/production-ping.env"
printf 'BACKUP_PING=%s\n' "$backup" > "$dir/production-backup.env"
for file in "$dir/production-ping.env" "$dir/production-backup.env"; do
  chown root:root "$file"
  chmod 0600 "$file"
done
systemctl daemon-reload
systemctl try-restart filedrawer-production.service

echo "pings: done"
