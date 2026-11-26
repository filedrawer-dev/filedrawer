#!/usr/bin/env bash
# ENVIRONMENT's GitHub token, staging's or production's, which the link checks call
# GitHub's API with (design.md, Submission and screening, step 2): 5,000 calls an
# hour instead of 60. Each environment has a token of its own, in a file of its own
# (#123), so staging's binary never holds production's, and a token one environment
# holds is refused for the other. Reads the token as the first line of stdin, into
# $TOKEN; an empty one keeps the token stored (#112), checked as a new one would be.
# Runs as root; safe to rerun, and the service restarts only when its token changed.
# Exits 2 for an environment it doesn't know.
set -euo pipefail
case "${ENVIRONMENT-}" in
  staging) other=production stage=github ;;
  production) other=staging stage="production github" ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
TOKEN="${TOKEN-}"
dir="$root/etc/filedrawer"
file="$dir/$ENVIRONMENT-github.env"

# stored FILE: the token FILE holds, or nothing.
stored() {
  sed -n "s/^FILEDRAWER_GITHUB_TOKEN='\(.*\)'\$/\1/p" "$1" 2>/dev/null || true
}

[[ -n "$TOKEN" ]] || TOKEN=$(stored "$file")
# systemd reads single-quoted values literally, so the token may hold anything but a
# single quote or a line break.
if [[ ! "$TOKEN" =~ ^(github_pat_|ghp_) || "$TOKEN" == *"'"* || "$TOKEN" == *$'\n'* ]]; then
  echo "no usable token for $ENVIRONMENT: it must start with github_pat_ (or ghp_) and hold no single quote or line break; to give one, run ops/provision-vps --ask-all $stage" >&2
  exit 1
fi
if [[ "$TOKEN" == "$(stored "$dir/$other-github.env")" ]]; then
  echo "that is $other's token; $ENVIRONMENT needs one of its own: make one, then run ops/provision-vps --ask-all $stage" >&2
  exit 1
fi

mkdir -p "$dir"
umask 077
printf "FILEDRAWER_GITHUB_TOKEN='%s'\n" "$TOKEN" > "$file.new"
unset TOKEN
# A rerun that keeps the token leaves the service running.
if cmp -s "$file.new" "$file"; then
  rm -f "$file.new"
  echo "github token ($ENVIRONMENT): unchanged"
  exit 0
fi
mv -f "$file.new" "$file"
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
# try-restart fails on a unit that doesn't exist, so only one service.sh wrote.
if [[ -e "$root/etc/systemd/system/filedrawer-$ENVIRONMENT.service" ]]; then
  systemctl try-restart "filedrawer-$ENVIRONMENT.service"
fi

echo "github token ($ENVIRONMENT): done"
