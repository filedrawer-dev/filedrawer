#!/usr/bin/env bash
# The GitHub token the link checks call GitHub's API with (design.md, Submission and
# screening, step 2): 5,000 calls an hour instead of 60. One file for every service
# on this server, staging's and production's (the owner's decision, 2026-11-07), and
# when it changes, each unit set up on the server is restarted to read it. Reads the
# token as the first line of stdin, into $TOKEN; an empty one keeps the token stored
# (#112), checked as a new one would be. Runs as root; safe to rerun.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
TOKEN="${TOKEN-}"
file="$root/etc/filedrawer/github.env"

if [[ -z "$TOKEN" ]]; then
  TOKEN=$(sed -n "s/^FILEDRAWER_GITHUB_TOKEN='\(.*\)'\$/\1/p" "$file" 2>/dev/null || true)
fi
# systemd reads single-quoted values literally, so the token may hold anything but a
# single quote or a line break.
if [[ ! "$TOKEN" =~ ^(github_pat_|ghp_) || "$TOKEN" == *"'"* || "$TOKEN" == *$'\n'* ]]; then
  echo "no usable token: it must start with github_pat_ (or ghp_) and hold no single quote or line break; to give one, run ops/provision-vps --ask-all github" >&2
  exit 1
fi

mkdir -p "$root/etc/filedrawer"
umask 077
printf "FILEDRAWER_GITHUB_TOKEN='%s'\n" "$TOKEN" > "$file.new"
unset TOKEN
# A rerun that keeps the token leaves the services running.
if cmp -s "$file.new" "$file"; then
  rm -f "$file.new"
  echo "github token: unchanged"
  exit 0
fi
mv -f "$file.new" "$file"
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
