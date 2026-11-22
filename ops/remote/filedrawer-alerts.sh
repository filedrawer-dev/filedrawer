# shellcheck shell=bash disable=SC2154  # the caller sets root, now, tries, pause and the alert_ names
# What the server's own jobs share (design.md, Hosting and operations): their mail to
# the owner, their healthchecks.io ping, and the check of the sites through Caddy.
# caddy-upgrade-check and filedrawer-reboot-check source it from
# /usr/local/lib/filedrawer-alerts.sh, where ops/provision-vps installs it before
# either. Sourcing it reads the settings below into globals and starts $problems
# empty; it runs no check and sends nothing until called.
#
# The caller sets, before sourcing:
#   alert_name    the job's name, for the journal and the Message-ID
#   alert_title   the mail's subject prefix, such as "Caddy guard"
#   root          the fake root tests give (FILEDRAWER_ROOT), else empty
#   now           the time, in seconds
#   tries, pause  how often and how far apart each request through Caddy is tried
#
# Settings: /etc/filedrawer/caddy-upgrades.env (ALERT_TO, PING), which
# caddy-upgrades.sh writes, and the SMTP settings ops/remote/mail.sh wrote for
# production, else staging. Secrets reach curl on stdin, never a command line, and
# the staging key, the SMTP password and the ping URL are taken out of all it mails,
# pings or prints. A mail it can't send, or no one to send it to, goes in
# $problems, which the caller reports.

caddyfile="$root/etc/caddy/Caddyfile"
sites="$root/etc/caddy/sites"
settings="$root/etc/filedrawer/caddy-upgrades.env"
problems=()

# setting FILE NAME: NAME's value in a KEY=VALUE file, single quotes undone.
setting() {
  local value
  value=$(sed -n "s/^$2=//p" "$1" 2>/dev/null | tail -n 1)
  [[ "$value" == \'*\' ]] && value=${value:1:${#value}-2}
  printf '%s' "$value"
}

alert_to=$(setting "$settings" ALERT_TO)
ping_url=$(setting "$settings" PING)
[[ "$ping_url" =~ ^https://hc-ping\.com/[A-Za-z0-9_./-]+$ ]] || ping_url=""
mail_env="$root/etc/filedrawer/production-mail.env"
[[ -f "$mail_env" ]] || mail_env="$root/etc/filedrawer/staging-mail.env"
smtp_host=$(setting "$mail_env" FILEDRAWER_SMTP_HOST)
smtp_port=$(setting "$mail_env" FILEDRAWER_SMTP_PORT)
smtp_user=$(setting "$mail_env" FILEDRAWER_SMTP_USER)
smtp_password=$(setting "$mail_env" FILEDRAWER_SMTP_PASSWORD)
mail_from=$(setting "$mail_env" FILEDRAWER_MAIL_FROM)
staging_key=$(sed -nE '/X-Staging-Key "[0-9a-f]{64}"/{s/.*X-Staging-Key "([0-9a-f]{64})".*/\1/p;q}' \
  "$sites/staging.caddy" 2>/dev/null)

# redact: stdin with the secrets taken out, by value and by shape.
redact() {
  local text secret
  text=$(cat)
  for secret in "$staging_key" "$smtp_password" "$ping_url"; do
    [[ -n "$secret" ]] && text=${text//"$secret"/[redacted]}
  done
  # shellcheck disable=SC2016  # a bcrypt hash's literal dollar signs, not expansions
  sed -E 's/[0-9a-f]{64}/[redacted]/g; s/\$2[aby]\$[0-9]{2}\$[./A-Za-z0-9]{53}/[redacted]/g' <<< "$text"
}

# say TEXT: to the journal, redacted.
say() { redact <<< "$alert_name: $*" >&2; }

# quoted VALUE: VALUE as a double-quoted string for a curl config.
quoted() {
  local value=${1//\\/\\\\}
  printf '"%s"' "${value//\"/\\\"}"
}

# send_mail SUBJECT BODY: sends to ALERT_TO through the SMTP settings; a mail it can't
# send is a problem of its own.
send_mail() {
  if [[ -z "$alert_to" || -z "$smtp_host" ]]; then
    say "no ALERT_TO or SMTP settings, so this went unmailed: $1"
    problems+=("no ALERT_TO or SMTP settings, so a mail went unsent: $1")
    return 1
  fi
  local message status=0
  message=$(mktemp)
  printf 'From: %s\r\nTo: %s\r\nSubject: %s: %s\r\nDate: %s\r\nMessage-ID: <%s.%s.%s@filedrawer.dev>\r\n\r\n%s\r\n' \
    "$mail_from" "$alert_to" "$alert_title" "$1" "$(date -R -d "@$now")" "$alert_name" "$now" "$$" \
    "$(redact <<< "$2" | sed 's/$/\r/')" > "$message"
  printf 'user = %s\n' "$(quoted "$smtp_user:$smtp_password")" |
    curl -sS -m 60 --ssl-reqd --url "smtps://$smtp_host:${smtp_port:-465}" \
      --mail-from "$(sed -E 's/.*<(.*)>.*/\1/' <<< "$mail_from")" \
      --mail-rcpt "$alert_to" -T "$message" -K - > /dev/null 2>&1 || status=1
  rm -f "$message"
  (( status == 0 )) || problems+=("the mail '$1' couldn't be sent")
  return "$status"
}

# ping_check SUFFIX BODY: tells healthchecks.io, when a ping URL is set.
ping_check() {
  [[ -n "$ping_url" ]] || return 0
  printf 'url = "%s%s"\n' "$ping_url" "$1" |
    curl -fsS -m 10 --retry 5 --retry-all-errors --data-raw "$(redact <<< "$2")" -K - \
      > /dev/null 2>&1 || say "the ping to healthchecks.io failed"
}

# through_caddy HOST PATH [KEY]: curl's "<code> <redirect>" for a request through Caddy.
through_caddy() {
  local host=$1 path=$2 key=${3-}
  {
    printf 'url = "https://%s%s"\n' "$host" "$path"
    [[ -n "$key" ]] && printf 'header = "X-Staging-Key: %s"\n' "$key"
  } | curl -s -o /dev/null -m 10 --resolve "$host:443:127.0.0.1" \
    -w '%{http_code} %{redirect_url}' -K - 2>/dev/null
}

backend_up() {
  [[ "$(curl -s -o /dev/null -m 5 -w '%{http_code}' "http://127.0.0.1:$1/tags" 2>/dev/null)" == 200 ]]
}

# expect HOST PATH WANT [KEY]: the request through Caddy answers WANT ("<code>
# <redirect>"), within the tries; else notes what it got in $got.
expect() {
  local n
  for (( n = 1; n <= tries; n++ )); do
    got=$(through_caddy "$1" "$2" "${4-}")
    [[ "${got% }" == "${3% }" ]] && return 0
    (( n < tries )) && sleep "$pause"
  done
  got="https://$1$2 answered '${got% }', not '${3% }'"
  return 1
}

# check_site ENV: appends to judged, unjudged or failed.
check_site() {
  local env=$1 port host got=""
  [[ -f "$sites/$env.caddy" ]] || return 0
  if [[ $env == production ]]; then port=8080; else port=8081; fi
  if [[ $env == staging && -z "$staging_key" ]]; then
    unjudged+=("$env can't be judged: no staging key found in its site file")
    return 0
  fi
  if ! backend_up "$port"; then
    unjudged+=("$env can't be judged: its server doesn't answer on 127.0.0.1:$port")
    return 0
  fi
  if [[ $env == production ]]; then
    expect filedrawer.dev /tags "200 " &&
      expect www.filedrawer.dev / "301 https://filedrawer.dev/"
  else
    expect staging.filedrawer.dev /robots.txt "200 " &&
      expect staging.filedrawer.dev /tags "401 " &&
      expect staging.filedrawer.dev /tags "200 " "$staging_key"
  fi || {
    if backend_up "$port"; then
      failed+=("$env: $got")
    else
      unjudged+=("$env can't be judged: its server stopped answering on 127.0.0.1:$port during the check")
    fi
    return 0
  }
  judged+=("$env")
}

# check [restart]: validate, optionally restart Caddy, and check each site. True
# when nothing failed; the findings are in judged, unjudged and failed, as text in
# $findings.
check() {
  local out
  judged=() unjudged=() failed=()
  if ! out=$(caddy validate --config "$caddyfile" --adapter caddyfile 2>&1); then
    failed+=("caddy validate refused the Caddyfile:
$(tail -n 20 <<< "$out")")
  else
    [[ "${1-}" == restart ]] && systemctl restart caddy
    check_site production
    check_site staging
  fi
  findings=""
  (( ${#judged[@]} )) && findings+="Checked and passing: ${judged[*]}."$'\n'
  local line
  for line in ${unjudged[@]+"${unjudged[@]}"} ${failed[@]+"${failed[@]}"}; do
    findings+="$line"$'\n'
  done
  (( ${#failed[@]} == 0 ))
}
