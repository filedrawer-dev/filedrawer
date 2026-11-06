#!/usr/bin/env bash
# Makes the owner's staging account the admin, so its browser session reaches the
# review queue at /admin (design.md, Admin). ADMIN_ACCOUNT is the ID the account page
# shows. Runs as root; safe to rerun. Exits 2 for an ID that isn't one, and 3 when
# staging has no binary yet, so the wizard can say to deploy first.
set -euo pipefail

# The override is for tests, which point it at a fake server root.
root="${FILEDRAWER_ROOT:-}"
bin="$root/srv/filedrawer/staging/bin/filedrawer"
database="$root/srv/filedrawer/staging/data/filedrawer.db"
account="${ADMIN_ACCOUNT-}"

if [[ ! "$account" =~ ^a-[0-9a-z]+$ ]]; then
  echo "'$account' isn't an account ID; copy it from the account page (a-...)" >&2
  exit 2
fi
if [[ ! -x "$bin" ]]; then
  echo "staging has no binary at $bin yet" >&2
  exit 3
fi

runuser -u filedrawer -- env FILEDRAWER_DATABASE="$database" "$bin" admin grant --account "$account"
