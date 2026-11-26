# shellcheck shell=bash disable=SC2034  # library.sh reads TOTAL_STAGES

# Production (#40): `ops/provision-vps production` sets up filedrawer.dev beside
# staging on the same server, each step a script in ops/remote/ run with
# ENVIRONMENT=production; `ops/provision-vps production account` comes after the
# first deploy (#42). Production's values are saved under PROD_ names in
# ops/.vps.env, so staging's are never offered for it.
PRODUCTION_NAMES=("filedrawer.dev" "www.filedrawer.dev" "files.filedrawer.dev")
# The settings production refuses to start without, and the stage that writes each.
PRODUCTION_NEEDS=(
  "/etc/filedrawer/production-mail.env:mail"
  "/etc/filedrawer/production-screening.env:screening"
  "/etc/filedrawer/production-ping.env:healthchecks.io"
  "/etc/filedrawer/production-github.env:GitHub token"
)
production_preflight_stage() {
  stage "Preflight: reach the server"
  VPS_HOST=$(_existing VPS_HOST || true)
  if [[ -z "$VPS_HOST" ]]; then
    warn "$ENV_FILE has no VPS_HOST. Production shares staging's server: run the"
    warn "staging wizard (ops/provision-vps) first."
    exit 1
  fi
  reach_server
  pause
}

production_dns_stage() {
  stage "DNS: point filedrawer.dev at the server"
  local name waiting=() records=("${PRODUCTION_NAMES[@]}")
  for name in "${PRODUCTION_NAMES[@]}"; do
    resolves "$name" || waiting+=("$name")
  done
  # A rerun lists only the names that don't resolve yet.
  (( ASK_ALL )) || records=("${waiting[@]}")
  if (( ${#records[@]} > 0 )); then
    say "In Cloudflare's DNS for filedrawer.dev, add these records if they aren't there,"
    say "each set to 'DNS only' (the gray cloud), not 'Proxied' (design.md, Hosting and"
    say "operations):"
    open_url "https://dash.cloudflare.com/"
    for name in "${records[@]}"; do
      step "$name  A     $VPS_HOST"
      [[ -n "$VPS_IPV6" ]] && step "$name  AAAA  $VPS_IPV6"
    done
    pause "Press Enter once the records are saved"
  fi
  (( ${#waiting[@]} == 0 )) || wait_to_resolve "${waiting[@]}"
  say "${PRODUCTION_NAMES[*]} resolve. ✓"
  pause
}

production_service_stage() {
  stage "Production's user, service and loopback rules"
  say "A 'filedrawer-prod' system user, /srv/filedrawer/production with a data folder"
  say "only it reads, and filedrawer-production.service on 127.0.0.1:8080, which waits"
  say "for its first binary (#42). Only Caddy and root may connect to that port, and"
  say "each service may bind only its own."
  remote service-user.sh ENVIRONMENT=production
  remote loopback.sh
  remote service.sh ENVIRONMENT=production
  # The install script that knows production, for #42's deploys.
  install_on_server filedrawer-install /usr/local/sbin
  pause
}

production_caddy_stage() {
  stage "Caddy: filedrawer.dev, files and www"
  say "filedrawer.dev and files.filedrawer.dev, open to all, and www redirecting to"
  say "filedrawer.dev. Until the first deploy they answer a short 503 that says to try"
  say "again later. Staging's sites stay as they are."
  remote caddy.sh ENVIRONMENT=production
  pause
}

production_mail_stage() {
  stage "Mail through Purelymail"
  local staging_sender keep=0
  staging_sender=$(_existing SMTP_USER || true)
  keep_secret "Purelymail password" /etc/filedrawer/production-mail.env "production mail" && keep=1
  if (( ! keep )); then
    say "Production sends verification and password-reset email as a Purelymail user"
    say "of its own, so staging's password can be revoked without touching it."
    open_url "https://purelymail.com/manage/users"
    local not_staging=""
    [[ -z "$staging_sender" ]] || not_staging="; not staging's sending user, $staging_sender"
    step "Add a user for production's mail, with a password of its own that holds no"
    step "single quote$not_staging."
    step "Then, on Purelymail's routing page, forward contact@filedrawer.dev, the address"
    step "the terms and privacy pages name, to your own inbox."
  fi
  PROD_SMTP_USER="" PROD_MAIL_FROM="" SMTP_PASSWORD=""
  if ! server_keeps PROD_SMTP_USER "$keep"; then
    ask PROD_SMTP_USER "Production's sending user (the SMTP login):"
    if [[ -n "$staging_sender" && "$PROD_SMTP_USER" == "$staging_sender" ]]; then
      warn "That is staging's sending user; production needs one of its own. Rerun to try again."
      exit 1
    fi
    write_env PROD_SMTP_USER "$PROD_SMTP_USER"
  fi
  if ! server_keeps PROD_MAIL_FROM "$keep"; then
    ask PROD_MAIL_FROM "The From line, such as File Drawer <noreply@filedrawer.dev>:"
    write_env PROD_MAIL_FROM "$PROD_MAIL_FROM"
  fi
  # mail.sh keeps a password only for the user it was stored with.
  (( keep )) || ask_new_secret SMTP_PASSWORD "The sending user's password:"
  remote_with_secret mail.sh SMTP_PASSWORD "$SMTP_PASSWORD" ENVIRONMENT=production \
    "SMTP_USER=$PROD_SMTP_USER" "MAIL_FROM=$PROD_MAIL_FROM"
  unset SMTP_PASSWORD
  if (( ! keep )); then
    note "The sign-up check that this mail reaches an inbox comes after the first deploy:"
    note "  ops/provision-vps production account"
  fi
  pause
}

# anthropic_accepts KEY: whether Anthropic's API takes KEY, from a call that costs
# nothing (listing models). The key goes to curl on stdin, never in a command line.
anthropic_accepts() {
  local status
  status=$(printf 'header = "x-api-key: %s"\nheader = "anthropic-version: 2023-06-01"\n' "$1" \
    | curl -K - -s -o /dev/null -w '%{http_code}' --max-time 20 \
        https://api.anthropic.com/v1/models || true)
  [[ "$status" == 200 ]]
}

production_screening_stage() {
  stage "Screening with the Anthropic API"
  local keep=0
  keep_secret "Anthropic API key" /etc/filedrawer/production-screening.env "production screening" && keep=1
  if (( ! keep )); then
    say "Production screens entries on an API key from a workspace of its own, within"
    say "the server's daily cap, and refuses to start without one."
    open_url "https://console.anthropic.com/settings/workspaces"
    step "Make a workspace for production alone, and set its monthly spend limit to"
    step "about \$40: above the most a \$1 daily cap can spend in a month, so the limit"
    step "only catches a cap that failed."
    pause "Press Enter once the workspace is made"
    open_url "https://console.anthropic.com/settings/keys"
    step "Make a key in that workspace, named such as 'filedrawer production'."
  fi
  if server_keeps PROD_SCREENING_DAILY_USD "$keep"; then
    PROD_SCREENING_DAILY_USD=""
  else
    ask PROD_SCREENING_DAILY_USD "Production's screening cap, in dollars a day (Enter for 1):" || PROD_SCREENING_DAILY_USD=""
    PROD_SCREENING_DAILY_USD="${PROD_SCREENING_DAILY_USD:-1}"
    write_env PROD_SCREENING_DAILY_USD "$PROD_SCREENING_DAILY_USD"
  fi
  API_KEY=""
  if (( ! keep )); then
    ask_new_secret API_KEY "The API key (sk-ant-...):"
    step "Checking the key with Anthropic..."
    if ! anthropic_accepts "$API_KEY"; then
      unset API_KEY
      warn "Anthropic refused the key, or didn't answer. Check it in the console and rerun:"
      warn "  ops/provision-vps production screening"
      exit 1
    fi
  fi
  remote_with_secret screening.sh API_KEY "$API_KEY" ENVIRONMENT=production \
    "DAILY_USD=$PROD_SCREENING_DAILY_USD"
  unset API_KEY
  pause
}

# production_github_stage: production's own GitHub token for the link checks (#123);
# production refuses to start without one.
production_github_stage() {
  stage "Production's GitHub token for the link checks"
  github_token_setup production "production github"
  pause
}

# production_github_oauth_stage: production's own GitHub OAuth App, for GitHub
# sign-in (#119); production starts without it, with GitHub sign-in off.
production_github_oauth_stage() {
  stage "GitHub sign-in: production's OAuth App"
  github_oauth_setup production https://filedrawer.dev PROD_ "production github-oauth"
  pause
}

# ping_url URL: whether URL is a healthchecks.io ping URL, as the server checks it.
ping_url() {
  [[ "$1" =~ ^https://hc-ping\.com/[A-Za-z0-9_./-]+$ ]]
}

production_pings_stage() {
  stage "healthchecks.io: the daily email and the nightly copy"
  local asked=() which
  DAILY_PING="" BACKUP_PING=""
  keep_secret "daily email's ping URL" /etc/filedrawer/production-ping.env "production pings" ||
    asked+=(DAILY_PING)
  keep_secret "nightly copy's ping URL" /etc/filedrawer/production-backup.env "production pings" ||
    asked+=(BACKUP_PING)
  if (( ${#asked[@]} )); then
    say "healthchecks.io emails you when a job stops reporting. Production refuses to"
    say "start without the daily email's ping URL. The nightly copy's (#41) goes to the"
    say "timer the next stage installs."
    open_url "https://healthchecks.io/"
    step "Sign up (free), then add two checks. Name them 'filedrawer daily email' and"
    step "'filedrawer nightly copy', and change each one's schedule to:"
    step "  Period: 1 day. Grace: 2 hours."
    step "Copy each one's ping URL (https://hc-ping.com/...)."
  fi
  for which in "${asked[@]}"; do
    case $which in
      DAILY_PING) ask_secret DAILY_PING "The daily email's ping URL:" ;;
      BACKUP_PING) ask_secret BACKUP_PING "The nightly copy's ping URL:" ;;
    esac
  done
  for which in "${asked[@]}"; do
    if ! ping_url "${!which}"; then
      warn "The ${which%_PING} one isn't a healthchecks.io ping URL (https://hc-ping.com/...)."
      warn "Rerun to try again."
      exit 1
    fi
  done
  remote_with_secret ping.sh PINGS "$DAILY_PING"$'\t'"$BACKUP_PING"
  unset DAILY_PING BACKUP_PING
  pause
}

production_backup_stage() {
  stage "Backups: the nightly copy and Hetzner's"
  say "A timer copies production's database each night at 23:30 UTC into backups/"
  say "beside it, keeps the newest 2, and pings the nightly copy's check (#41). Until"
  say "the first deploy there's no database, so it skips and the check stays 'new'."
  # The program first: enabling the timer can start a missed run at once.
  install_on_server filedrawer-backup /usr/local/sbin
  remote backup.sh
  if (( ! ASK_ALL )) && [[ -n "$(_existing PROD_HETZNER_BACKUPS_SHOWN || true)" ]]; then
    note "Hetzner's backups were shown on an earlier run; --ask-all shows them again."
  else
    say "Hetzner's own backups hold the copies, the attached files and the config:"
    say "daily, 7 kept, for about 20% of the server's price."
    open_url "https://console.hetzner.cloud/"
    step "Open the server, then Backups, then Enable backups."
    write_env PROD_HETZNER_BACKUPS_SHOWN yes
  fi
  pause
}

# caddy_upgrades_stage: Caddy follows its releases, with a guard (#100): the server is
# one, so this serves staging and production alike.
caddy_upgrades_stage() {
  stage "Caddy upgrades, with a guard that rolls back"
  say "Caddy ships its security fixes beside breaking changes, so the server takes"
  say "each Caddy release, and a guard checks the sites after each one: if they fail,"
  say "it puts the version before back, holds it, and emails you."
  # The program first, with the library it sources: the units start it.
  install_upkeep_scripts
  local alert_before keep=0 holds=0 settings=/etc/filedrawer/caddy-upgrades.env
  alert_before=$(_existing CADDY_ALERT_TO || true)
  remote_has "$settings" && holds=1
  CADDY_ALERT_TO=""
  if ! server_keeps CADDY_ALERT_TO "$holds"; then
    ask CADDY_ALERT_TO "The address the guard emails [Enter for ops@filedrawer.dev]:" || CADDY_ALERT_TO=""
    CADDY_ALERT_TO=${CADDY_ALERT_TO:-ops@filedrawer.dev}
    if [[ ! "$CADDY_ALERT_TO" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]+$ ]]; then
      warn "That isn't one email address. Rerun to try again."
      exit 1
    fi
    write_env CADDY_ALERT_TO "$CADDY_ALERT_TO"
  fi
  CADDY_PING=""
  if keep_secret "guard's ping URL" "$settings" caddy-upgrades; then
    keep=1
  else
    say "healthchecks.io tells you when the guard fails, or stops running, even when"
    say "mail through Purelymail doesn't arrive."
    open_url "https://healthchecks.io/"
    step "Add a check named 'filedrawer caddy guard'. Period: 1 day. Grace: 2 hours."
    step "Have its alerts go to an address that isn't at filedrawer.dev, since that mail"
    step "goes through Purelymail too. Copy its ping URL (https://hc-ping.com/...)."
    ask_secret CADDY_PING "The guard's ping URL:"
    if ! ping_url "$CADDY_PING"; then
      warn "That isn't a healthchecks.io ping URL (https://hc-ping.com/...). Rerun to try again."
      exit 1
    fi
  fi
  remote_with_secret caddy-upgrades.sh SETTINGS "$CADDY_ALERT_TO"$'\t'"$CADDY_PING"
  unset CADDY_PING
  # Something new to test: a ping URL, or an address.
  if (( ! keep )) || [[ "$CADDY_ALERT_TO" != "$alert_before" ]]; then
    ssh -o BatchMode=yes "root@$VPS_HOST" "/usr/local/sbin/caddy-upgrade-check --test-mail" < /dev/null ||
      warn "The test mail failed to send; what the server said is above."
    if ! confirm "Did a 'Caddy guard: a test' email reach $CADDY_ALERT_TO, and does the check show a ping?"; then
      warn "Check the address and the ping URL, then rerun: ops/provision-vps --ask-all caddy-upgrades"
      SKIPPED+=("Caddy guard: the test mail or ping didn't arrive")
    fi
  fi
  pause
}

# install_upkeep_scripts: the library the guard and the reboot check source, and both
# programs, so a change to the library never reaches one without the other.
install_upkeep_scripts() {
  install_on_server filedrawer-alerts.sh /usr/local/lib
  install_on_server caddy-upgrade-check /usr/local/sbin
  install_on_server filedrawer-reboot-check /usr/local/sbin
}

# reboots_stage: the server reboots itself for a kernel it installed, with notice;
# one server, so this serves staging and production alike.
reboots_stage() {
  stage "Reboots for new kernels, with notice"
  say "unattended-upgrades installs new kernels but never reboots. When one waits for"
  say "a reboot, the server emails you, at the guard's address, naming it and when it"
  say "will reboot itself: the first 05:00 UTC at least 4 days later. Then it emails"
  say "that it is back, with the services and sites. It waits, and puts the reboot off"
  say "a day, while a deploy or backup runs or someone is logged in."
  local new=1
  remote_has /etc/systemd/system/filedrawer-reboot-check-daily.timer && new=0
  # The programs first: the units start one.
  install_upkeep_scripts
  if ! remote reboots.sh; then
    warn "The reboot check's setup failed; the server said why above."
    SKIPPED+=("reboots: setup failed; fix, then rerun ops/provision-vps reboots")
    pause
    return 0
  fi
  if (( new || ASK_ALL )); then
    step "What it finds now (a dry run, which changes nothing):"
    ssh -o BatchMode=yes "root@$VPS_HOST" "/usr/local/sbin/filedrawer-reboot-check --dry-run" < /dev/null ||
      warn "The test mail failed to send; what the server said is above."
    if ! confirm "Did a 'Reboot check: a test' email arrive?"; then
      warn "It mails as the Caddy guard does: check with ops/provision-vps --ask-all caddy-upgrades"
      SKIPPED+=("reboots: the test mail didn't arrive")
    fi
  fi
  pause
}

production_check_stage() {
  stage "Check filedrawer.dev from outside"
  local tags www
  step "https://filedrawer.dev/tags should answer 503 until the first deploy, then 200..."
  # curl waits out a 503's Retry-After (120 s) between retries; --retry-max-time
  # keeps the whole check to about half a minute.
  tags=$(curl -s -o /dev/null -w '%{http_code}' --retry 5 --retry-delay 5 --retry-all-errors \
    --retry-max-time 30 https://filedrawer.dev/tags || true)
  step "https://www.filedrawer.dev/ should redirect..."
  www=$(curl -s -o /dev/null -w '%{http_code}' https://www.filedrawer.dev/ || true)
  if [[ ( "$tags" == 503 || "$tags" == 200 ) && "$www" == 301 ]]; then
    say "/tags $tags, www $www. ✓"
  else
    warn "Got /tags $tags and www $www (expected 503 or 200, and 301). On the server,"
    warn "'journalctl -u caddy -n 50' shows why, often a certificate still being issued."
    SKIPPED+=("production HTTPS check: /tags $tags, www $www")
  fi
  local need path missing=()
  for need in "${PRODUCTION_NEEDS[@]}"; do
    path="${need%%:*}"
    remote_has "$path" || missing+=("$need")
  done
  if (( ${#missing[@]} > 0 )); then
    warn "Production won't start without these, and the first deploy (#42) would have"
    warn "no binary to roll back to. Rerun ops/provision-vps production for:"
    for need in "${missing[@]}"; do
      warn "  ${need%%:*}  (${need#*:})"
      SKIPPED+=("production setting ${need%%:*}")
    done
  fi
  note "After the first deploy (#42), run this straight away, since until an admin"
  note "exists the daily email reports to healthchecks.io as failed each day:"
  note "  ops/provision-vps production account"
  pause
}

production_account_stage() {
  stage "Your production account, its mail, and the uptime check"
  need_vps_host
  if ! ssh -o BatchMode=yes "root@$VPS_HOST" "test -x /srv/filedrawer/production/bin/filedrawer" < /dev/null; then
    warn "Production has no server yet. Once the first deploy (#42) puts it there, run:"
    warn "  ops/provision-vps production account"
    SKIPPED+=("production account: run 'ops/provision-vps production account' after the first deploy")
    pause
    return 0
  fi
  say "Sign up on production with a Gmail address, so you see where a stranger's"
  say "verification mail lands."
  open_url "https://filedrawer.dev/signup"
  step "Find the verification email, and click its verification link."
  if ! confirm "Did the verification email reach Gmail's inbox, not spam?"; then
    say "Mail that lands in spam needs filedrawer.dev's records in order:"
    open_url "https://purelymail.com/manage/domains"
    step "SPF: one TXT record on filedrawer.dev, v=spf1 include:_spf.purelymail.com ~all,"
    step "and no second SPF record."
    step "DKIM: the CNAME records Purelymail lists for filedrawer.dev, each 'DNS only'."
    step "DMARC: the DMARC record Purelymail lists, and no other on _dmarc.filedrawer.dev."
    step "Wait for Purelymail to show each as valid. Gmail's 'Show original' on the"
    step "message says whether SPF, DKIM and DMARC passed."
    warn "Then sign up again with another Gmail address and rerun:"
    warn "  ops/provision-vps production account"
    exit 1
  fi
  # A rerun takes a saved ID, so it needs no page to copy it from.
  if (( ASK_ALL )) || [[ -z "$(_existing PROD_ADMIN_ACCOUNT || true)" ]]; then
    open_url "https://filedrawer.dev/account"
    step "Copy the account ID the page shows (a-...)."
  fi
  ask PROD_ADMIN_ACCOUNT "Your production account ID:"
  if [[ ! "$PROD_ADMIN_ACCOUNT" =~ ^a-[0-9a-z]+$ ]]; then
    warn "That isn't an account ID (a- then lowercase letters and digits). Rerun to try again."
    exit 1
  fi
  write_env PROD_ADMIN_ACCOUNT "$PROD_ADMIN_ACCOUNT"
  if ! remote admin-grant.sh ENVIRONMENT=production "ADMIN_ACCOUNT=$PROD_ADMIN_ACCOUNT"; then
    warn "Granting failed; the server said why above. Rerun to try again."
    exit 1
  fi
  say "Your account is production's admin. ✓"
  say "Now an outside check that emails you when the site is down."
  open_url "https://uptimerobot.com/"
  step "Sign up (free), then add an HTTP(s) monitor of https://filedrawer.dev/tags,"
  step "every 5 minutes, alerting your email."
  step "A deploy restarts the server for a few seconds; if the monitor offers a delay"
  step "before it alerts, a few minutes keeps a deploy from paging you."
  pause
}

# production [STAGE]: production's stages; or one alone, mail, screening, github,
# github-oauth or pings;
# or account, the one after the first deploy.
production() {
  case "${1:-}" in
    "")
      TOTAL_STAGES=13
      banner "File Drawer VPS: production setup"
      production_preflight_stage
      production_dns_stage
      production_service_stage
      production_caddy_stage
      production_mail_stage
      production_screening_stage
      production_github_stage
      production_github_oauth_stage
      production_pings_stage
      production_backup_stage
      caddy_upgrades_stage
      reboots_stage
      outbound_check production
      production_check_stage
      ;;
    account)
      TOTAL_STAGES=1
      production_account_stage
      ;;
    mail | screening | github | github-oauth | pings)
      TOTAL_STAGES=1
      need_vps_host
      "production_${1//-/_}_stage"
      outbound_check production
      ;;
    *)
      warn "ops/provision-vps production takes no stage, or one of account, mail, screening,"
      warn "github, github-oauth and pings, as in: ops/provision-vps production account"
      exit 2
      ;;
  esac
  finish
}
