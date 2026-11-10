#!/usr/bin/env bash
# Names staging's smoke-test account to the server, in its own settings file: that
# account's entries aren't screened, so CI's smoke test calls no model after each
# deploy (FILEDRAWER_SMOKE_ACCOUNT; production refuses it). SMOKE_ACCOUNT comes as a
# variable. SCREEN_TEST_ACCOUNT, when set, names the screen-test account too
# (FILEDRAWER_SCREEN_TEST_ACCOUNT, not held to an account's share of the screening
# cap), and CI_SCREEN_ACCOUNT the CI screen account (FILEDRAWER_CI_SCREEN_ACCOUNT, no
# daily allowance). Runs as root; safe to rerun. Exits 2 for an ID that isn't one.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
account="${SMOKE_ACCOUNT-}"
screen_test="${SCREEN_TEST_ACCOUNT-}"
ci_screen="${CI_SCREEN_ACCOUNT-}"
for id in "$account" ${screen_test:+"$screen_test"} ${ci_screen:+"$ci_screen"}; do
  if [[ ! "$id" =~ ^a-[0-9a-z]+$ ]]; then
    echo "'$id' isn't an account ID (a-...)" >&2
    exit 2
  fi
done

file="$root/etc/filedrawer/staging-smoke.env"
umask 077
{
  printf 'FILEDRAWER_SMOKE_ACCOUNT=%s\n' "$account"
  # The account ops/smoke-staging eval submits as: not held to an account's share.
  [[ -z "$screen_test" ]] || printf 'FILEDRAWER_SCREEN_TEST_ACCOUNT=%s\n' "$screen_test"
  # The account CI's screen check submits as: no daily allowance.
  [[ -z "$ci_screen" ]] || printf 'FILEDRAWER_CI_SCREEN_ACCOUNT=%s\n' "$ci_screen"
} > "$file"
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
systemctl try-restart filedrawer-staging.service

echo "smoke account setting: done"
