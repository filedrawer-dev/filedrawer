#!/usr/bin/env bash
# Checks each secret the server holds with the service that takes it, unattended,
# so a rerun of ops/provision-vps that keeps a secret still shows whether it works
# (#112): the Anthropic API keys (a free list of models), each environment's GitHub
# token (its rate limit, which names its expiry), its GitHub OAuth client secret (a
# code exchange with a made-up code, which GitHub refuses after checking the app's
# credentials), and the healthchecks.io ping URLs (their /log endpoint, which
# records an event and leaves the check's state alone). The SMTP
# logins aren't here: each run that restarts a service probes them as the service's
# own user (filedrawer-install check-outbound).
#
# Prints a line per secret held: "ok: WHAT: ...", "accepted: WHAT: ..." (an
# Anthropic key, which a workspace out of credit would pass too), "expiring:
# WHAT: ...", "failed: WHAT: ..." or "unchecked: WHAT: ...", the last when the
# service didn't say; and "missing: WHAT: ..." for an environment set up without
# its GitHub token (#123), which counts as failed for production, since it won't
# start without one. The lines speak of the secret alone, never of the way out,
# since root's curl can get out where a service can't. A failed or expiring line
# names the wizard stage that replaces it. No secret is shown or put in a command
# line: each reaches curl in its config, on stdin. Runs as root; writes nothing.
# Exits 1 when a secret failed.
set -euo pipefail

# The overrides are for tests, which point them at a fake root and a fixed clock.
root="${FILEDRAWER_ROOT:-}"
dir="$root/etc/filedrawer"
now="${FILEDRAWER_NOW:-$(date +%s)}"
failed=0
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# value FILE NAME: NAME's value in FILE, single quotes undone, or nothing.
value() {
  [[ -f "$dir/$1" ]] || return 0
  sed -n "s/^$2=//p" "$dir/$1" | sed "s/^'\(.*\)'\$/\1/"
}

# quoted TEXT: TEXT as a double-quoted curl config value.
quoted() {
  local text=${1//\\/\\\\}
  printf '"%s"' "${text//\"/\\\"}"
}

# call CONFIG: runs curl with CONFIG on stdin, the body in $work/body and the headers
# in $work/headers; prints the HTTP status, which curl's -w gives as 000 when
# nothing answered.
call() {
  local status
  : > "$work/body"
  : > "$work/headers"
  status=$(printf '%s\n' "$1" |
    curl -sS -K - -o "$work/body" -D "$work/headers" -w '%{http_code}' --max-time 20 \
      2>/dev/null) || true
  printf '%s' "${status:-000}"
}

# unanswered WHAT SERVICE STATUS: the unchecked line for a service that answered
# STATUS, or didn't answer at all.
unanswered() {
  if [[ "$3" == 000 ]]; then
    report unchecked "$1" "$2 didn't answer"
  else
    report unchecked "$1" "$2 answered $3"
  fi
}

# header NAME: NAME's value in the last call's headers.
header() {
  sed -n "s/^$1: *//Ip" "$work/headers" | tr -d '\r' | tail -n 1
}

# report STATUS WHAT DETAIL [STAGE]: one line; a failure counts.
report() {
  local line="$1: $2: $3"
  [[ -z "${4:-}" || "$1" == ok ]] ||
    line+=". To replace it: ops/provision-vps --ask-all $4"
  printf '%s\n' "$line"
  [[ "$1" != failed ]] || failed=1
}

anthropic() {
  local env="$1" what="$2" stage="$3" key status
  key=$(value "$env-screening.env" FILEDRAWER_ANTHROPIC_API_KEY)
  [[ -n "$key" ]] || return 0
  status=$(call "url = $(quoted https://api.anthropic.com/v1/models)
header = $(quoted "x-api-key: $key")
header = $(quoted "anthropic-version: 2023-06-01")")
  case $status in
    # A workspace out of credit still answers 200, so it says no more than this.
    200) report accepted "$what" "Anthropic takes it" ;;
    401 | 403) report failed "$what" "Anthropic refused it" "$stage" ;;
    *) unanswered "$what" Anthropic "$status" ;;
  esac
}

# github ENV STAGE: ENV's GitHub token (#123), through its rate limit, which costs no
# call.
github() {
  local what="$1's GitHub token" stage="$2" token status limit expires left
  token=$(value "$1-github.env" FILEDRAWER_GITHUB_TOKEN)
  if [[ -z "$token" ]]; then
    # A server that ran on the token both environments shared before #123 has none
    # for staging until the github stage runs again.
    if [[ -e "$root/etc/systemd/system/filedrawer-$1.service" ]]; then
      report missing "$what" "the server has none; to set one: ops/provision-vps $stage"
      [[ "$1" != production ]] || failed=1
    fi
    return 0
  fi
  status=$(call "url = $(quoted https://api.github.com/rate_limit)
header = $(quoted "Authorization: Bearer $token")
header = $(quoted "User-Agent: filedrawer-check-secrets")")
  case $status in
    200)
      limit=$(header x-ratelimit-limit)
      # GitHub answers 200 without a token too, allowing 60 calls an hour, so such
      # an answer, or one without the header, says nothing of the token.
      if [[ ! "$limit" =~ ^[0-9]+$ ]] || (( limit <= 60 )); then
        report unchecked "$what" "GitHub answered as to a call without a token"
        return 0
      fi
      expires=$(header github-authentication-token-expiration)
      if [[ -n "$expires" ]] && left=$(date -d "$expires" +%s 2>/dev/null) &&
        (( left - now < 14 * 86400 )); then
        report expiring "$what" "it expires on ${expires%% *}; make a new one" "$stage"
      else
        report ok "$what" "accepted, ${limit} calls an hour"
      fi
      ;;
    401) report failed "$what" "GitHub refused it (expired or revoked)" "$stage" ;;
    *) unanswered "$what" GitHub "$status" ;;
  esac
}

# github_oauth ENV WHAT STAGE: ENV's OAuth App, through the code exchange with a code
# no one was given. GitHub answers 200 with an error either way: it checks the
# client ID and secret first (incorrect_client_credentials), then the code
# (bad_verification_code), so the second means the secret works (#119).
github_oauth() {
  local file="$1-github-oauth.env" what="$2" stage="$3" id secret status error
  id=$(value "$file" FILEDRAWER_GITHUB_CLIENT_ID)
  secret=$(value "$file" FILEDRAWER_GITHUB_CLIENT_SECRET)
  [[ -n "$secret" ]] || return 0
  status=$(call "url = $(quoted https://github.com/login/oauth/access_token)
header = $(quoted "Accept: application/json")
header = $(quoted "User-Agent: filedrawer-check-secrets")
data-urlencode = $(quoted "client_id=$id")
data-urlencode = $(quoted "client_secret=$secret")
data-urlencode = $(quoted "code=filedrawer-check-secrets")")
  error=$(sed -n 's/.*"error" *: *"\([a-z_]*\)".*/\1/p' "$work/body" | tail -n 1)
  case "$status:$error" in
    200:bad_verification_code) report ok "$what" "GitHub takes it with client ID $id" ;;
    200:incorrect_client_credentials)
      report failed "$what" "GitHub refused it with client ID $id" "$stage" ;;
    200:*) report unchecked "$what" "GitHub answered ${error:-something else}" ;;
    *) unanswered "$what" GitHub "$status" ;;
  esac
}

ping_check() {
  local file="$1" name="$2" what="$3" stage="$4" url status body
  url=$(value "$file" "$name")
  [[ -n "$url" ]] || return 0
  # A URL that already ends in a suffix (/fail, /start, /0) or a slash can't take /log.
  if [[ "$url" =~ /(fail|start|log|[0-9]+)?$ ]]; then
    report unchecked "$what" "it ends in a suffix, so /log can't be added" "$stage"
    return 0
  fi
  status=$(call "url = $(quoted "$url/log")
data = $(quoted "ops/provision-vps: checking the URL")")
  body=$(tr -d '\r\n' < "$work/body")
  if [[ "$status" == 200 && "$body" == OK ]]; then
    report ok "$what" "healthchecks.io knows it"
  elif [[ "$status" == 404 || "$body" == "OK (not found)" ]]; then
    report failed "$what" "healthchecks.io doesn't know it" "$stage"
  elif [[ "$status" == 000 ]]; then
    unanswered "$what" healthchecks.io "$status"
  else
    report unchecked "$what" "healthchecks.io answered $status${body:+ ${body:0:40}}"
  fi
}

anthropic staging "staging's Anthropic API key" screening
anthropic production "production's Anthropic API key" "production screening"
github staging github
github production "production github"
github_oauth staging "staging's GitHub OAuth client secret" github-oauth
github_oauth production "production's GitHub OAuth client secret" "production github-oauth"
ping_check production-ping.env FILEDRAWER_DAILY_EMAIL_PING "the daily email's ping URL" "production pings"
ping_check production-backup.env BACKUP_PING "the nightly copy's ping URL" "production pings"
ping_check caddy-upgrades.env PING "the Caddy guard's ping URL" caddy-upgrades
exit "$failed"
