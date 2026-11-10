#!/usr/bin/env bash
# The GitHub token the link checks call GitHub's API with (design.md, Submission and
# screening, step 2): 5,000 calls an hour instead of 60. One file for every service
# on this server, staging's and production's (the owner's decision, 2026-11-07), and
# each unit set up on the server is restarted to read it. Reads the token as the
# first line of stdin, into $TOKEN. Runs as root; safe to rerun, and a rerun replaces
# the token.
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
mkdir -p "$root/etc/filedrawer"
umask 077
printf "FILEDRAWER_GITHUB_TOKEN='%s'\n" "$TOKEN" > "$file"
unset TOKEN
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
# try-restart fails on a unit that doesn't exist, so only the ones service.sh wrote.
for environment in staging production; do
  if [[ -e "$root/etc/systemd/system/filedrawer-$environment.service" ]]; then
    systemctl try-restart "filedrawer-$environment.service"
  fi
done

echo "github token: done"
