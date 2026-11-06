#!/usr/bin/env bash
# The account CI's smoke test submits as (ops/smoke-staging), and a new API token for
# it. Runs as root; safe to rerun. SMOKE_ACCOUNT names the account an earlier run made:
# it is kept when staging still has it, and made again when not. Each run adds a
# token, and the wizard stores only the newest.
# Prints "<account ID><TAB><token>" as the only line on stdout, for the wizard to read.
# Exits 3 when staging has no binary yet, so the wizard can say to deploy first.
set -euo pipefail

# The override is for tests, which point it at a fake server root.
root="${FILEDRAWER_ROOT:-}"
bin="$root/srv/filedrawer/staging/bin/filedrawer"
database="$root/srv/filedrawer/staging/data/filedrawer.db"

if [[ ! -x "$bin" ]]; then
  echo "staging has no binary at $bin yet" >&2
  exit 3
fi

# admin ARGS...: filedrawer admin, as the service user, on staging's database.
admin() {
  runuser -u filedrawer -- env FILEDRAWER_DATABASE="$database" "$bin" admin "$@"
}

new_token() {
  admin create-token --account "$1" --name "smoke test (CI)"
}

account="${SMOKE_ACCOUNT:-}"
token=""
if [[ -n "$account" ]]; then
  errors=$(mktemp)
  trap 'rm -f "$errors"' EXIT
  if ! token=$(new_token "$account" 2> "$errors"); then
    # Only a missing account means making a new one; any other error stops here.
    if ! grep -q "no account" "$errors"; then
      cat "$errors" >&2
      exit 1
    fi
    echo "staging has no account $account any more; making a new one" >&2
    token=""
  fi
fi
if [[ -z "$token" ]]; then
  account=$(admin create-account --display-name "Smoke test (CI)")
  token=$(new_token "$account")
fi
printf '%s\t%s\n' "$account" "$token"
