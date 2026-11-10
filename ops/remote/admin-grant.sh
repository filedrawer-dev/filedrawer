#!/usr/bin/env bash
# Makes the owner's account on ENVIRONMENT, staging or production, the admin, so its
# browser session reaches the review queue at /admin (design.md, Admin), and gets the
# daily email. ADMIN_ACCOUNT is the ID the account page shows. Runs as root; safe to
# rerun. Exits 2 for an environment or an ID it doesn't know, and 3 when the
# environment has no binary yet, so the wizard can say to deploy first.
set -euo pipefail

case "${ENVIRONMENT-}" in
  staging) user=filedrawer ;;
  production) user=filedrawer-prod ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
# The override is for tests, which point it at a fake server root.
root="${FILEDRAWER_ROOT:-}"
bin="$root/srv/filedrawer/$ENVIRONMENT/bin/filedrawer"
database="$root/srv/filedrawer/$ENVIRONMENT/data/filedrawer.db"
account="${ADMIN_ACCOUNT-}"

if [[ ! "$account" =~ ^a-[0-9a-z]+$ ]]; then
  echo "'$account' isn't an account ID; copy it from the account page (a-...)" >&2
  exit 2
fi
if [[ ! -x "$bin" ]]; then
  echo "$ENVIRONMENT has no binary at $bin yet" >&2
  exit 3
fi

runuser -u "$user" -- env FILEDRAWER_DATABASE="$database" "$bin" admin grant --account "$account"
