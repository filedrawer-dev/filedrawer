# shellcheck shell=bash
# Staging's stages, in the order ops/provision-vps runs them, then the ones it also
# runs alone (smoke, screening, github, github-oauth, mail and admin), then the
# GitHub helpers only they use. Production's stages, caddy-upgrades included, are in
# production.sh.

staging_preflight_stage() {
  stage "Preflight: reach the server"
  say "Your SSH key must already log you in as root (Hetzner adds the key you chose"
  say "when you created the server). Nothing here asks for or stores a key."
  ask VPS_HOST "The server's IPv4 address:"
  write_env VPS_HOST "$VPS_HOST"
  reach_server
  pause
}

staging_dns_stage() {
  stage "DNS: point the names at the server"
  records=("filedrawer.dev" "www.filedrawer.dev" "staging.filedrawer.dev" "files.filedrawer.dev" "files.staging.filedrawer.dev")
  if (( ! ASK_ALL )); then
    # Only the served names that don't resolve yet; nothing to do when all do.
    waiting=()
    for name in "staging.filedrawer.dev" "files.staging.filedrawer.dev"; do
      resolves "$name" || waiting+=("$name")
    done
    records=("${waiting[@]}")
  else
    waiting=("staging.filedrawer.dev")
  fi
  if (( ${#records[@]} > 0 )); then
    say "In Cloudflare's DNS for filedrawer.dev, add these records, each set to"
    say "'DNS only' (the gray cloud), not 'Proxied' (design.md, Hosting and operations):"
    open_url "https://dash.cloudflare.com/"
    for name in "${records[@]}"; do
      step "$name  A     $VPS_HOST"
      [[ -n "$VPS_IPV6" ]] && step "$name  AAAA  $VPS_IPV6"
    done
    note "Only staging and its files host are served for now. Caddy asks for certificates only for the"
    note "sites it serves, so the other names can sit idle until launch, when www"
    note "redirects to filedrawer.dev."
    pause "Press Enter once the records are saved"
  fi
  wait_to_resolve "${waiting[@]}"
  if (( ${#waiting[@]} > 0 )); then
    say "${waiting[*]} resolve. ✓"
  else
    say "Every served name resolves. ✓"
  fi
  pause
}

base_system_stage() {
  stage "Base system"
  say "Updates, automatic security updates, the firewall (SSH, HTTP, HTTPS), a 1 GB"
  say "swapfile, journald capped at 200 MB and 14 days, and SSH logins by key only."
  warn "This turns off password logins over SSH. The preflight showed your key works."
  # A rerun runs it again unasked once it has run: it writes this file.
  if { (( ! ASK_ALL )) && remote_has /etc/ssh/sshd_config.d/10-filedrawer.conf; } ||
    confirm "Run it on $VPS_HOST?"; then
    remote base.sh
  else
    SKIPPED+=("base system (ops/remote/base.sh)")
  fi
  pause
}

service_user_stage() {
  stage "Service user and directories"
  say "A 'filedrawer' system user with no shell, /srv/filedrawer/staging, and"
  say "/etc/filedrawer for settings."
  remote service-user.sh ENVIRONMENT=staging
  pause
}

staging_service_stage() {
  stage "The staging service"
  say "filedrawer-staging.service, enabled. It waits until a deploy puts the binary"
  say "at /srv/filedrawer/staging/bin/filedrawer, then serves on 127.0.0.1:8081."
  remote service.sh ENVIRONMENT=staging
  pause
}

staging_caddy_stage() {
  stage "Caddy and basic auth"
  say "Caddy serves staging.filedrawer.dev over HTTPS and asks for a user name and"
  say "password on every request, reads included. Keep the password in your password"
  say "manager: the wizard never saves it, and the server keeps only its hash."
  say "Agents get in with a staging key instead, sent as an X-Staging-Key header, since"
  say "their API token needs the Authorization header that basic auth uses."
  ask STAGING_USER "Basic-auth user name:"
  write_env STAGING_USER "$STAGING_USER"
  # caddy.sh keeps the server's hash for an empty password, but only for its own user.
  STAGING_PASSWORD=""
  keep_secret "basic-auth password" /etc/caddy/sites/staging.caddy ||
    ask_new_secret STAGING_PASSWORD "Basic-auth password:"
  if [[ "$STAGING_PASSWORD" == *$'\t'* ]]; then
    warn "The password holds a tab, which the wizard can't send. Rerun with another."
    exit 1
  fi
  STAGING_KEY=$(_existing STAGING_KEY || true)
  if [[ ! "$STAGING_KEY" =~ ^[0-9a-f]{64}$ ]]; then
    STAGING_KEY=$(od -An -tx1 -N32 /dev/urandom | tr -d ' \n')
  fi
  write_env STAGING_KEY "$STAGING_KEY"
  remote_with_secret caddy.sh STAGING_SECRETS "$STAGING_PASSWORD"$'\t'"$STAGING_KEY" \
    ENVIRONMENT=staging "STAGING_USER=$STAGING_USER"
  unset STAGING_PASSWORD
  # Only Caddy and root may open the apps' ports, so no local user skips the key;
  # it names the user caddy, so it comes after Caddy's install.
  remote loopback.sh
  note "The staging key is in $ENV_FILE. Give it to agents with their API token."
  pause
}

deploy_key_stage() {
  stage "Deploy key for GitHub Actions"
  say "A key that lets GitHub Actions upload a new staging binary, and nothing else:"
  say "the server runs only filedrawer-receive for it, which checks the upload's hash"
  say "and installs it with a health check and rollback. The private half goes straight"
  say "into the repo's secrets through gh, and is deleted from this machine."
  need_gh
  install_on_server filedrawer-receive /usr/local/bin
  install_on_server filedrawer-install /usr/local/sbin
  # The key is a secret of the `staging` environment, which only main may deploy to, so
  # a workflow edited on another branch can't read it.
  step "Creating the 'staging' environment, open to deployments from main only..."
  gh api -X PUT "repos/{owner}/{repo}/environments/staging" --input - > /dev/null <<'JSON'
{"deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}
JSON
  gh api -X POST "repos/{owner}/{repo}/environments/staging/deployment-branch-policies" \
    -f name=main -f type=branch > /dev/null 2>&1 || true   # already there on a rerun
  if staging_has_secret DEPLOY_SSH_KEY &&
    { (( ! ASK_ALL )) || ! confirm "Make a new deploy key? CI's current one then stops working."; }; then
    # The rest of deploy-user.sh still runs, so a newer sudo rule reaches the server.
    remote deploy-user.sh "DEPLOY_PUBLIC_KEY="
    say "DEPLOY_SSH_KEY is set already; kept. ✓ To replace it: ops/provision-vps --ask-all"
    KEPT+=("the deploy key, DEPLOY_SSH_KEY. To replace it: ops/provision-vps --ask-all")
  else
    key_dir=$(mktemp -d)
    trap 'rm -rf "$key_dir"' EXIT
    ssh-keygen -q -t ed25519 -N '' -C "filedrawer-deploy@github-actions" -f "$key_dir/key"
    remote deploy-user.sh "DEPLOY_PUBLIC_KEY=$(cat "$key_dir/key.pub")"
    gh secret set DEPLOY_SSH_KEY --env staging < "$key_dir/key"
    WRITTEN_SECRET+=("DEPLOY_SSH_KEY (environment staging)")
    rm -rf "$key_dir"
  fi
  set_var VPS_HOST "$VPS_HOST"
  # The host key comes over the root login the preflight already trusts, never from a
  # scan that would trust whoever answers first.
  host_key=$(ssh -o BatchMode=yes "root@$VPS_HOST" cat /etc/ssh/ssh_host_ed25519_key.pub < /dev/null | cut -d' ' -f1,2)
  set_var VPS_KNOWN_HOSTS "$VPS_HOST $host_key"
  note "The next push to main deploys staging once CI passes."
  pause
}

staging_check_stage() {
  stage "Check staging from outside"
  step "https://staging.filedrawer.dev/robots.txt should answer 200 with no password..."
  robots=$(curl -s -o /dev/null -w '%{http_code}' --retry 5 --retry-delay 5 --retry-all-errors \
    https://staging.filedrawer.dev/robots.txt || true)
  step "https://staging.filedrawer.dev/ should answer 401 without the password..."
  root=$(curl -s -o /dev/null -w '%{http_code}' https://staging.filedrawer.dev/ || true)
  if [[ "$robots" == 200 && "$root" == 401 ]]; then
    say "robots.txt 200, everything else 401. ✓"
  else
    warn "Got robots.txt $robots and / $root (expected 200 and 401). On the server,"
    warn "'journalctl -u caddy -n 50' shows why, often a certificate still being issued."
    SKIPPED+=("staging HTTPS check: robots.txt $robots, / $root")
  fi
  step "With the staging key, /tags should get past Caddy..."
  keyed=$(curl -s -o /dev/null -w '%{http_code}' -H "X-Staging-Key: $STAGING_KEY" \
    https://staging.filedrawer.dev/tags || true)
  case "$keyed" in
    200) say "The staging key works, and the server answers. ✓" ;;
    502) say "The staging key works; the server answers once it is deployed. ✓" ;;
    *) warn "With the staging key, /tags answered $keyed."
       SKIPPED+=("staging key check: /tags answered $keyed") ;;
  esac
  note "With the password, / answers 502 until the first deploy starts the server."
  note "That happens on the next push to main once CI passes."
  pause
}

# ci_account ROLE SAVED SECRET WHAT: the CI account for ROLE (ops/remote/ci-account.sh)
# and a token, made on the server; its ID saved as SAVED in ops/.vps.env and its token
# stored as the staging secret SECRET, never shown. Both are kept when set, unless the
# owner asks for a new token (asked only under --ask-all). WHAT names it in messages.
# Returns 1 when the server fails to make it, having said why.
ci_account() {
  local role=$1 saved=$2 secret=$3 what=$4 account reply status=0
  local account_and_token=$'^a-[^[:space:]]+\tfdt_[^[:space:]]+$'
  account=$(_existing "$saved" || true)
  if [[ -n "$account" ]] && staging_has_secret "$secret" \
    && { ! (( ASK_ALL )) || ! confirm "$secret is set already. Make a new token anyway?"; }; then
    note "Kept the token in $secret. To replace it: ops/provision-vps --ask-all smoke"
    KEPT+=("$secret: kept. To replace it: ops/provision-vps --ask-all smoke")
    return 0
  fi
  step "Making the $what and a token on $VPS_HOST..."
  reply=$(remote ci-account.sh "ROLE=$role" "ACCOUNT=$account") || status=$?
  if (( status == 3 )); then
    not_deployed_yet smoke "$what"
  elif (( status != 0 )) || [[ ! "$reply" =~ $account_and_token ]]; then
    warn "Making the $what failed; the server said why above. Rerun to try again."
    return 1
  else
    write_env "$saved" "${reply%%$'\t'*}"
    set_staging_secret "$secret" "${reply#*$'\t'}"
  fi
}

# smoke_stage: the accounts CI submits as after each deploy (.github/workflows/ci.yml:
# smoke-staging and screen-staging), and the secrets those jobs read.
smoke_stage() {
  stage "CI's accounts: the smoke test and the screen check"
  say "After each deploy, CI runs ops/smoke-staging against staging, as an agent would,"
  say "as an account of its own. Its entries say SMOKE TEST in the title, and stay. It"
  say "then runs ops/smoke-staging screen, which checks that staging screens with the"
  say "real model, as a second account, the CI screen account, apart from the"
  say "screen-test account the eval uses."
  say "This makes both accounts and their tokens on the server, and stores the tokens"
  say "and the staging key as secrets of the 'staging' environment. None is shown."
  need_gh
  STAGING_KEY=$(_existing STAGING_KEY || true)
  if [[ ! "$STAGING_KEY" =~ ^[0-9a-f]{64}$ ]]; then
    warn "$ENV_FILE has no staging key. Run the whole wizard (ops/provision-vps) first."
    exit 1
  fi
  ci_account smoke SMOKE_ACCOUNT SMOKE_TEST_TOKEN "smoke-test account" || exit 1
  # If the screen account fails, a smoke account just made is still named to staging
  # below, so the next deploy's smoke test isn't screened; then the stage stops.
  local screen_failed=0
  ci_account screen CI_SCREEN_ACCOUNT CI_SCREEN_TOKEN "CI screen account" || screen_failed=1
  SMOKE_ACCOUNT=$(_existing SMOKE_ACCOUNT || true)
  CI_SCREEN_ACCOUNT=$(_existing CI_SCREEN_ACCOUNT || true)
  if [[ -n "$SMOKE_ACCOUNT" ]]; then
    # Staging doesn't screen the smoke test's entries, so the smoke test calls no
    # model. The unit first, so it reads the file smoke-env.sh writes.
    remote service.sh ENVIRONMENT=staging
    # The screen-test account, if ops/.screen-test.env names one, isn't held to an
    # account's share of the day's screening cap (ops/smoke-staging eval).
    local screen_test
    screen_test=$(screen_test_setting SCREEN_TEST_ACCOUNT 'a-[0-9a-z]\{1,\}')
    # The CI screen account has no daily allowance, since each deploy submits as it.
    remote smoke-env.sh "SMOKE_ACCOUNT=$SMOKE_ACCOUNT" ${screen_test:+"SCREEN_TEST_ACCOUNT=$screen_test"} \
      ${CI_SCREEN_ACCOUNT:+"CI_SCREEN_ACCOUNT=$CI_SCREEN_ACCOUNT"}
  fi
  if (( screen_failed )); then
    exit 1
  fi
  set_staging_secret STAGING_KEY "$STAGING_KEY"
  # CI's screen check read the eval's token until it got an account of its own.
  if staging_has_secret SCREEN_TEST_TOKEN; then
    note "Nothing reads the staging secret SCREEN_TEST_TOKEN any more: CI's screen check"
    note "has an account of its own. Delete it:"
    note "  gh secret delete SCREEN_TEST_TOKEN --env staging"
  fi
  note "The next deploy runs the smoke test. To run it from here instead:"
  note "  FILEDRAWER_STAGING_KEY=... FILEDRAWER_STAGING_API_KEY=... ops/smoke-staging"
  pause
}

# screening_stage: staging's own Anthropic API key and daily cap (design.md, Submission
# and screening). Until it runs, staging publishes everything unscreened.
screening_stage() {
  stage "Screening with the Anthropic API"
  local keep=0
  keep_secret "Anthropic API key" /etc/filedrawer/staging-screening.env screening && keep=1
  if (( ! keep )); then
    say "Staging screens entries with Haiku and Sonnet on an API key of its own, billed to"
    say "you, within a daily cap. Until this runs, staging publishes everything unscreened."
    open_url "https://console.anthropic.com/settings/keys"
    step "Make a key for staging alone, named such as 'filedrawer staging'. A workspace"
    step "with its own monthly spend limit is a second guard beside the server's cap."
  fi
  if server_keeps SCREENING_DAILY_USD "$keep"; then
    SCREENING_DAILY_USD=""
  else
    ask SCREENING_DAILY_USD "Staging's screening cap, in dollars a day (Enter for 1):" || SCREENING_DAILY_USD=""
    SCREENING_DAILY_USD="${SCREENING_DAILY_USD:-1}"
    write_env SCREENING_DAILY_USD "$SCREENING_DAILY_USD"
  fi
  API_KEY=""
  (( keep )) || ask_new_secret API_KEY "The API key (sk-ant-...):"
  # The unit first, so it reads the file screening.sh writes.
  remote service.sh ENVIRONMENT=staging
  remote_with_secret screening.sh API_KEY "$API_KEY" ENVIRONMENT=staging "DAILY_USD=$SCREENING_DAILY_USD"
  unset API_KEY
  if (( ! keep )); then
    note "The key is on the server only. To check it, submit an entry: it should come back"
    note "public, rejected or review, and journalctl -u filedrawer-staging logs each call."
  fi
  pause
}

# github_stage: the GitHub token the link checks call GitHub's API with (design.md,
# Submission and screening, step 2), in one file that staging reads now and
# production will at launch. Until it runs, staging gets 60 API calls an hour, which
# the check spends when GitHub's commit page lists no branch or doesn't answer.
github_stage() {
  stage "A GitHub token for the link checks"
  # The unit first, so it reads the file github-token.sh writes.
  remote service.sh ENVIRONMENT=staging
  if keep_secret "GitHub token" /etc/filedrawer/github.env github; then
    remote_with_secret github-token.sh TOKEN ""
    pause
    return 0
  fi
  github_stage_body
}

# github_stage_body: asks for the GitHub token and sends it to the server, which
# restarts every unit that reads it.
github_stage_body() {
  say "Submissions ask GitHub whether a code link's commit is on one of the repository's"
  say "own branches or tags: its commit page first, then its API when the page lists"
  say "none or doesn't answer. Without a token the API allows 60 calls an hour; with"
  say "one, 5,000. The server spends at most half, and leaves the rest to you."
  open_url "https://github.com/settings/personal-access-tokens/new"
  step "Make a fine-grained token named such as 'filedrawer link checks'."
  step "Repository access: 'Public repositories'. Add no permissions: it only reads."
  step "Expiration: the longest offered. Put the date in your calendar to renew it."
  ask_new_secret TOKEN "The token (github_pat_...):"
  remote_with_secret github-token.sh TOKEN "$TOKEN"
  unset TOKEN
  note "The token is on the server only, in one file staging and production both read."
  note "Until it is there, staging logs 'no FILEDRAWER_GITHUB_TOKEN' at each start, and"
  note "production won't start."
  pause
}

# github_oauth_stage: staging's own GitHub OAuth App, for GitHub sign-in (#119).
github_oauth_stage() {
  stage "GitHub sign-in: staging's OAuth App"
  github_oauth_setup staging https://staging.filedrawer.dev "" github-oauth
  pause
}

# mail_stage: staging's Purelymail sender.
mail_stage() {
  stage "Mail through Purelymail"
  local keep=0
  keep_secret "Purelymail password" /etc/filedrawer/staging-mail.env mail && keep=1
  if (( ! keep )); then
    say "Staging sends verification and password-reset email through Purelymail SMTP."
    say "Purelymail must know filedrawer.dev, and the registrar needs the DNS records it"
    say "asks for, or mail lands in spam."
    open_url "https://purelymail.com/manage/domains"
    step "Add filedrawer.dev as a domain, then add every DNS record Purelymail lists for"
    step "it (MX, SPF, DKIM and DMARC) at your registrar, and wait for Purelymail to show"
    step "them as valid."
    step "Add a user for sending, such as noreply@filedrawer.dev, with a password of its"
    step "own (Purelymail's 'app password' works too). It mustn't contain a single quote."
    note "The server connects to smtp.purelymail.com on port 465 (TLS). Check that against"
    note "Purelymail's own SMTP settings page if it has changed."
  fi
  SMTP_USER="" MAIL_FROM="" SMTP_PASSWORD=""
  if ! server_keeps SMTP_USER "$keep"; then
    ask SMTP_USER "The sending user's address (the SMTP login):"
    write_env SMTP_USER "$SMTP_USER"
  fi
  if ! server_keeps MAIL_FROM "$keep"; then
    ask MAIL_FROM "The From line, such as File Drawer <noreply@filedrawer.dev>:"
    write_env MAIL_FROM "$MAIL_FROM"
  fi
  # mail.sh keeps a password only for the user it was stored with.
  (( keep )) || ask_new_secret SMTP_PASSWORD "The sending user's password:"
  remote_with_secret mail.sh SMTP_PASSWORD "$SMTP_PASSWORD" ENVIRONMENT=staging \
    "SMTP_USER=$SMTP_USER" "MAIL_FROM=$MAIL_FROM"
  unset SMTP_PASSWORD
  if (( ! keep )); then
    note "To test it later: sign up at https://staging.filedrawer.dev/signup and watch"
    note "for the verification email."
  fi
  pause
}

# not_deployed_yet NAME WHAT: says staging has no server yet, and how to run the
# stage NAME once it has.
not_deployed_yet() {
  warn "Staging has no server yet. Once a push to main deploys it, run:"
  warn "  ops/provision-vps $1"
  SKIPPED+=("$2: run 'ops/provision-vps $1' after the first deploy")
}

# admin_stage: makes your own staging account the admin, so its browser session
# reaches the review queue at /admin.
admin_stage() {
  stage "Your admin account"
  say "The review queue at https://staging.filedrawer.dev/admin answers only the admin's"
  say "browser session."
  if ! ssh -o BatchMode=yes "root@$VPS_HOST" "test -x /srv/filedrawer/staging/bin/filedrawer" < /dev/null; then
    not_deployed_yet admin "admin account"
    pause
    return 0
  fi
  # A rerun takes a saved ID, so it needs no page to copy it from.
  if (( ASK_ALL )) || [[ -z "$(_existing ADMIN_ACCOUNT || true)" ]]; then
    say "Sign up and verify on staging first if you haven't."
    open_url "https://staging.filedrawer.dev/account"
    step "Copy the account ID the page shows (a-...)."
  fi
  ask ADMIN_ACCOUNT "Your account ID:"
  if [[ ! "$ADMIN_ACCOUNT" =~ ^a-[0-9a-z]+$ ]]; then
    warn "That isn't an account ID (a- then lowercase letters and digits). Rerun to try again."
    exit 1
  fi
  write_env ADMIN_ACCOUNT "$ADMIN_ACCOUNT"
  local status=0
  remote admin-grant.sh ENVIRONMENT=staging "ADMIN_ACCOUNT=$ADMIN_ACCOUNT" || status=$?
  if (( status == 3 )); then
    not_deployed_yet admin "admin account"
  elif (( status != 0 )); then
    warn "Granting failed; the server said why above. Rerun to try again."
    exit 1
  else
    say "Your account is the admin. ✓"
  fi
  pause
}

# need_gh: stops the wizard unless gh is logged in.
need_gh() {
  if ! { command -v gh >/dev/null && gh auth status >/dev/null 2>&1; }; then
    warn "gh isn't logged in here. Run 'gh auth login', then rerun the wizard."
    exit 1
  fi
}

# staging_has_secret NAME: whether the `staging` environment holds secret NAME. A gh
# that fails stops the wizard: read as "absent", it would replace a working key.
staging_has_secret() {
  local names
  if ! names=$(gh secret list --env staging --json name --jq '.[].name' 2>/dev/null); then
    warn "gh secret list failed, so the wizard can't tell whether $1 is set. Rerun once gh works."
    exit 1
  fi
  [[ -n "$names" ]] && grep -qx "$1" <<<"$names"
}

# set_staging_secret NAME VALUE: sets NAME as a secret of the `staging` environment,
# with VALUE on stdin so it never sits in a command line.
set_staging_secret() {
  printf '%s' "$2" | gh secret set "$1" --env staging
  WRITTEN_SECRET+=("$1 (environment staging)")
  printf '  %s✓ set%s GitHub secret %s (environment staging)\n' "$GREEN" "$RESET" "$1"
}

# screen_test_setting NAME VALUE-PATTERN: NAME's value in ops/.screen-test.env,
# written as NAME='value', when it matches the sed regex VALUE-PATTERN; else nothing.
screen_test_setting() {
  sed -n "s/^$1='\($2\)'\$/\1/p" ops/.screen-test.env 2>/dev/null || true
}
