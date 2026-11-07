#!/usr/bin/env bash
# The GitHub token the link checks call GitHub's API with (design.md, Submission and
# screening, step 2): 5,000 calls an hour instead of 60. One file for every service
# on this server: staging's unit reads it now, and production's will at launch (the
# owner's decision, 2026-11-07). Reads the token as the first line of stdin, into
# $TOKEN. Runs as root; safe to rerun, and a rerun replaces the token.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
TOKEN="${TOKEN-}"

# systemd reads single-quoted values literally, so the token may hold anything but a
# single quote or a line break.
if [[ ! "$TOKEN" =~ ^(github_pat_|ghp_) || "$TOKEN" == *"'"* || "$TOKEN" == *$'\n'* ]]; then
  echo "the token must start with github_pat_ (or ghp_) and hold no single quote or line break" >&2
  exit 1
fi

file="$root/etc/filedrawer/github.env"
umask 027
printf "FILEDRAWER_GITHUB_TOKEN='%s'\n" "$TOKEN" > "$file"
unset TOKEN
chown root:filedrawer "$file"
chmod 0640 "$file"
systemctl daemon-reload
systemctl try-restart filedrawer-staging.service

echo "github token: done"
