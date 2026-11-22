#!/usr/bin/env bash
# ENVIRONMENT's GitHub OAuth App, staging's or production's, for GitHub sign-in
# (#119): each environment has an app of its own, in a file of its own, so
# production's client secret never reaches staging's binary (design.md, Hosting and
# operations), and service.sh never overwrites it. Without the file the server runs
# with GitHub sign-in off. Reads the client secret as the first line of stdin, into
# $CLIENT_SECRET; CLIENT_ID comes as a variable. An empty CLIENT_ID keeps the stored
# one, and an empty secret keeps the stored one for the same client ID, since a
# secret belongs to its app (#112). Runs as root; safe to rerun, and the service
# restarts only when the settings changed. Exits 2 for an environment it doesn't
# know.
set -euo pipefail
case "${ENVIRONMENT-}" in
  staging | production) ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
CLIENT_ID="${CLIENT_ID-}" CLIENT_SECRET="${CLIENT_SECRET-}"

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
file="$root/etc/filedrawer/$ENVIRONMENT-github-oauth.env"
stage=github-oauth
[[ $ENVIRONMENT == staging ]] || stage="production github-oauth"

stored_id=$(sed -n 's/^FILEDRAWER_GITHUB_CLIENT_ID=//p' "$file" 2>/dev/null || true)
[[ -n "$CLIENT_ID" ]] || CLIENT_ID="$stored_id"
# GitHub's client IDs are letters and digits (an older GitHub App's hold a dot), so
# the value needs no quoting.
if [[ ! "$CLIENT_ID" =~ ^[A-Za-z0-9.]+$ ]]; then
  echo "no usable client ID given, and none stored: it is letters and digits; run ops/provision-vps --ask-all $stage" >&2
  exit 1
fi
if [[ -z "$CLIENT_SECRET" && "$stored_id" == "$CLIENT_ID" ]]; then
  CLIENT_SECRET=$(sed -n "s/^FILEDRAWER_GITHUB_CLIENT_SECRET='\(.*\)'\$/\1/p" "$file" 2>/dev/null || true)
fi
# systemd reads single-quoted values literally, so the secret may hold anything but a
# single quote or a line break; GitHub's hold neither, nor a space. A kept secret
# never shows: the message names none.
if [[ -z "$CLIENT_SECRET" || ! "$CLIENT_SECRET" =~ ^[[:graph:]]+$ || "$CLIENT_SECRET" == *"'"* ]]; then
  echo "no usable client secret for $CLIENT_ID: it holds no space, single quote or line break; to give one, run ops/provision-vps --ask-all $stage" >&2
  exit 1
fi

mkdir -p "$root/etc/filedrawer"
umask 077
cat > "$file.new" <<CONF
FILEDRAWER_GITHUB_CLIENT_ID=${CLIENT_ID}
FILEDRAWER_GITHUB_CLIENT_SECRET='${CLIENT_SECRET}'
CONF
unset CLIENT_SECRET
# A rerun that keeps everything leaves the service running.
if cmp -s "$file.new" "$file"; then
  rm -f "$file.new"
  echo "GitHub OAuth App ($ENVIRONMENT): unchanged"
  exit 0
fi
mv -f "$file.new" "$file"
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
systemctl try-restart "filedrawer-$ENVIRONMENT.service"

echo "GitHub OAuth App ($ENVIRONMENT): done"
