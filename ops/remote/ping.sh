#!/usr/bin/env bash
# Production's healthchecks.io ping URLs (design.md, Hosting and operations): the daily
# email's, which the server reads as FILEDRAWER_DAILY_EMAIL_PING and without which
# production refuses to start, and the nightly copy's (#41), which its timer reads as
# BACKUP_PING from a file of its own. Reads "<daily URL><TAB><backup URL>" as the
# first line of stdin, into $PINGS: anyone holding a ping URL can report a run, so
# neither sits in a command line. Each must be an https URL on hc-ping.com with a
# path, as the server checks. An empty URL keeps the one stored (#112), checked as a
# new one would be; an empty line keeps both. Runs as root; safe to rerun, and
# production restarts only when a URL changed.
set -euo pipefail
: "${PINGS?pass '<daily URL><TAB><backup URL>' as the first line of stdin}"

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
dir="$root/etc/filedrawer"
ping_url='^https://hc-ping\.com/[A-Za-z0-9_./-]+$'
[[ -n "$PINGS" ]] || PINGS=$'\t'
if [[ "$PINGS" != *$'\t'* ]]; then
  echo "expected '<daily URL><TAB><backup URL>'" >&2
  exit 1
fi
daily="${PINGS%%$'\t'*}"
backup="${PINGS#*$'\t'}"
unset PINGS
[[ -n "$daily" ]] ||
  daily=$(sed -n 's/^FILEDRAWER_DAILY_EMAIL_PING=//p' "$dir/production-ping.env" 2>/dev/null || true)
[[ -n "$backup" ]] ||
  backup=$(sed -n 's/^BACKUP_PING=//p' "$dir/production-backup.env" 2>/dev/null || true)
# The URLs are secrets, so a refusal names which one, never its text.
for which in daily backup; do
  url="$daily"
  [[ $which == daily ]] || url="$backup"
  if [[ ! "$url" =~ $ping_url ]]; then
    echo "the $which ping isn't a healthchecks.io ping URL, such as https://hc-ping.com/<uuid>; to give one, run ops/provision-vps --ask-all production pings" >&2
    exit 1
  fi
done

mkdir -p "$dir"
umask 077
printf 'FILEDRAWER_DAILY_EMAIL_PING=%s\n' "$daily" > "$dir/production-ping.env.new"
printf 'BACKUP_PING=%s\n' "$backup" > "$dir/production-backup.env.new"
# A rerun that keeps both leaves production running.
if cmp -s "$dir/production-ping.env.new" "$dir/production-ping.env" &&
  cmp -s "$dir/production-backup.env.new" "$dir/production-backup.env"; then
  rm -f "$dir/production-ping.env.new" "$dir/production-backup.env.new"
  echo "pings: unchanged"
  exit 0
fi
for file in "$dir/production-ping.env" "$dir/production-backup.env"; do
  mv -f "$file.new" "$file"
  chown root:root "$file"
  chmod 0600 "$file"
done
systemctl daemon-reload
systemctl try-restart filedrawer-production.service

echo "pings: done"
