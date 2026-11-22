# shellcheck shell=bash
# ops/provision-vps's helpers for both environments: running scripts on the server,
# what it holds, keeping its secrets on a rerun, checking them, and probing its way
# out.

# remote SCRIPT [ENV=VALUE...]: runs ops/remote/SCRIPT as root on $VPS_HOST, with the
# settings given in its environment.
remote() {
  local script="$1"; shift
  ssh -o BatchMode=yes "root@$VPS_HOST" "$(remote_command "$@")" < "$REMOTE/$script"
}

# remote_with_secret SCRIPT NAME SECRET [ENV=VALUE...]: like remote, but SECRET goes
# as the first line of stdin, ahead of the script, and the remote shell reads it into
# $NAME. It never sits in a command line on either machine.
remote_with_secret() {
  local script="$1" name="$2" secret="$3"; shift 3
  { printf '%s\n' "$secret"; cat "$REMOTE/$script"; } \
    | ssh -o BatchMode=yes "root@$VPS_HOST" "IFS= read -r $name; export $name; $(remote_command "$@")"
}

# remote_command [ENV=VALUE...]: the remote command line that runs a script read
# from stdin with these settings. ssh joins its arguments into one line for the
# remote shell, so each setting is quoted; with none, it is plain `bash -s`.
remote_command() {
  local setting quoted=""
  for setting in "$@"; do
    quoted+="$(printf '%q' "$setting") "
  done
  if [[ -n "$quoted" ]]; then
    printf 'env %sbash -s' "$quoted"
  else
    printf 'bash -s'
  fi
}

# ask_new_secret KEY "Prompt": asks twice, and stops the wizard if the two differ or
# are empty.
ask_new_secret() {
  local again
  ask_secret "$1" "$2"
  ask_secret _AGAIN "The same again:"
  again="$_AGAIN"; unset _AGAIN
  if [[ -z "${!1}" || "${!1}" != "$again" ]]; then
    warn "The two differ or are empty. Rerun the wizard to try again."
    exit 1
  fi
}

# remote_has PATH: whether PATH exists on the server. An ssh that fails stops the
# wizard: read as "absent", it would ask for a secret the server holds.
remote_has() {
  local status=0
  ssh -o BatchMode=yes "root@$VPS_HOST" "test -f $1" < /dev/null || status=$?
  case $status in
    0) return 0 ;;
    1) return 1 ;;
  esac
  warn "ssh root@$VPS_HOST failed (exit $status) while looking for $1. Rerun once ssh works."
  exit 1
}

# What a rerun kept, with how to change each, and what check_secrets found, listed
# after finish's summary, since each stage clears the screen.
KEPT=()
CHECKED=()

# check_secrets: checks each secret the server holds with the service that takes it
# (ops/remote/check-secrets.sh, #112), so a kept secret still shows whether it
# works. What failed, expires soon or couldn't be checked goes in the list of what's
# left to do; it never stops the wizard.
check_secrets() {
  local output line status=0
  [[ -n "${VPS_HOST:-}" ]] || return 0
  step "Checking the secrets the server holds with the services that take them..."
  output=$(remote check-secrets.sh 2>&1) || status=$?
  while IFS= read -r line; do
    case $line in
      "") ;;
      ok:* | accepted:*) CHECKED+=("$line") ;;
      *) SKIPPED+=("$line") ;;
    esac
  done <<< "$output"
  if (( status > 1 )); then
    SKIPPED+=("the server's secrets: their check stopped (exit $status); rerun to check them")
  fi
}

eval "library_$(declare -f finish)"
finish() {
  check_secrets
  library_finish
  local line
  if (( ${#KEPT[@]} )); then
    note "kept, as the server or GitHub holds them:"
    for line in "${KEPT[@]}"; do note "  - $line"; done
    printf '\n'
  fi
  if (( ${#CHECKED[@]} )); then
    note "the server's secrets, checked:"
    for line in "${CHECKED[@]}"; do note "  - $line"; done
    printf '\n'
  fi
}

# server_keeps KEY KEEP: true in a rerun of a stage that keeps the server's settings
# (KEEP is 1) when ops/.vps.env lacks KEY. The stage then sends KEY empty and its
# remote script keeps the server's value, rather than take a default or a guess.
server_keeps() {
  (( $2 && ! ASK_ALL )) && [[ -z "$(_existing "$1" || true)" ]]
}

# need_vps_host: VPS_HOST from ops/.vps.env, or stops the wizard: a stage alone runs
# on a server the whole wizard set up.
need_vps_host() {
  VPS_HOST=$(_existing VPS_HOST || true)
  [[ -n "$VPS_HOST" ]] || { warn "$ENV_FILE has no VPS_HOST. Run the whole wizard first."; exit 1; }
}

# keep_secret WHAT PATH [STAGE]: true when the server holds the secret in PATH and
# it stays: the stage then sends it empty, and its remote script keeps its copy.
# Under --ask-all the owner is asked first, before any tab or instructions; answering
# yes means a new one, and an empty answer then is refused, never kept. STAGE names
# the wizard stage that changes it.
keep_secret() {
  remote_has "$2" || return 1
  if (( ASK_ALL )); then
    if confirm "Change the server's $1?"; then
      return 1
    fi
    note "Kept the server's $1."
  else
    say "The server's $1 is set; kept. ✓ To change it: ops/provision-vps --ask-all${3:+ $3}"
    KEPT+=("the $1. To change it: ops/provision-vps --ask-all${3:+ $3}")
  fi
}

# resolves NAME: whether NAME resolves to the server. getent's whole answer is read
# first, and awk reads all of it, so no reader stops early on a live pipe (#39).
resolves() {
  local answer
  answer=$(getent ahostsv4 "$1" || true)
  awk -v host="$VPS_HOST" '$1 == host { found = 1 } END { exit !found }' <<<"$answer"
}

# wait_to_resolve NAME...: waits until each NAME resolves to the server, with a hint
# when one resolves elsewhere, such as to a proxied Cloudflare record.
wait_to_resolve() {
  local name hinted elsewhere answer
  for name in "$@"; do
    step "Waiting for $name to resolve to $VPS_HOST (Ctrl-C to stop)..."
    hinted=""
    until resolves "$name"; do
      answer=$(getent ahostsv4 "$name" || true)
      elsewhere=$(awk 'NR == 1 {print $1}' <<<"$answer")
      if [[ -n "$elsewhere" && -z "$hinted" ]]; then
        printf '\n'
        warn "$name resolves to $elsewhere, not $VPS_HOST. If that is a"
        warn "Cloudflare address, the record is proxied: click its orange cloud to make it"
        warn "gray ('DNS only'). Still waiting..."
        hinted=yes
      fi
      sleep 15
      printf '.'
    done
    printf '\n'
  done
}

# reach_server: checks that root on $VPS_HOST takes your SSH key, and sets VPS_IPV6 to
# the server's IPv6 address, if it has one. ssh forwards its stdin, so each call reads
# /dev/null, not the answers to come.
reach_server() {
  step "Checking that 'ssh root@$VPS_HOST' works without a password..."
  if ! ssh -o BatchMode=yes -o ConnectTimeout=10 "root@$VPS_HOST" \
    'grep -E "^(ID|VERSION_ID)=" /etc/os-release' < /dev/null; then
    warn "ssh root@$VPS_HOST failed. Log in once by hand to accept the host key, then rerun."
    exit 1
  fi
  VPS_IPV6=$(ssh -o BatchMode=yes "root@$VPS_HOST" \
    "ip -6 -o addr show scope global | awk '{print \$4}' | cut -d/ -f1 | awk 'NR == 1'" < /dev/null)
  note "IPv6: ${VPS_IPV6:-none found}"
}

# install_on_server SCRIPT DIR: copies ops/remote/SCRIPT to DIR on the server, root's
# and executable.
install_on_server() {
  ssh -o BatchMode=yes "root@$VPS_HOST" \
    "install -o root -g root -m 0755 /dev/stdin $2/$1" < "$REMOTE/$1"
}

# outbound_check ENV: probes the way out of ENV's installed binary, as its service's
# user under its unit's sandbox (#102), once the scripts before have restarted it, so
# a unit or settings change that cuts the server off shows while you're here. It
# installs the current filedrawer-install first, which holds the probe. A failure
# warns and goes in the closing list; it never stops the wizard.
outbound_check() {
  local env="$1" output status=0
  step "Checking that $env's server reaches the hosts it calls..."
  if ! install_on_server filedrawer-install /usr/local/sbin; then
    warn "Couldn't install filedrawer-install on the server, so $env isn't probed."
    SKIPPED+=("$env's outbound probe: couldn't install filedrawer-install; rerun")
    return 0
  fi
  output=$(ssh -o BatchMode=yes "root@$VPS_HOST" \
    "/usr/local/sbin/filedrawer-install check-outbound $env" < /dev/null 2>&1) || status=$?
  local line
  while IFS= read -r line; do
    [[ -z "$line" ]] || note "$line"
  done <<< "$output"
  if (( status == 0 )); then
    # With no binary yet, or one from before the probe, the install says so instead.
    [[ "$output" != *"probe: "* ]] || say "$env's way out: checked. ✓"
    return 0
  fi
  warn "$env's server can't reach every host it calls (above), or the probe couldn't"
  warn "run: look at its unit and settings, and journalctl -u filedrawer-$env."
  # The closing screen clears this one, so its list keeps what failed: the install's
  # own line naming the hosts.
  local failed why="exit $status"
  failed=$(sed -n "s/^.*'s server can't reach: //p" <<< "$output")
  [[ -z "$failed" ]] || why+="; can't reach $failed"
  SKIPPED+=("$env's outbound probe failed ($why): fix, then rerun")
  pause
}

# github_oauth_setup ENV SITE PREFIX STAGE: ENV's own GitHub OAuth App, for GitHub
# sign-in (#119), at SITE. Each environment has an app of its own, so production's
# client secret never reaches staging's binary. It is optional: without it the site
# runs with GitHub sign-in off, and a no is saved as PREFIX"GITHUB_SIGNIN" in
# ops/.vps.env, so a rerun asks nothing and --ask-all asks again. A held app keeps
# the server's client ID unless --ask-all is given another, and a yes to changing
# its secret rotates it (ops/revoke-all.md). PREFIX is "" or PROD_, as for the
# other saved values; STAGE names the wizard stage.
github_oauth_setup() {
  local env="$1" site="$2" id_name="${3}GITHUB_CLIENT_ID" off_name="${3}GITHUB_SIGNIN" \
    stage="$4" file="/etc/filedrawer/$1-github-oauth.env" held=0 keep=0 id=""
  local name="File Drawer" off="GitHub sign-in ($1): off. To set it up: ops/provision-vps --ask-all $4"
  [[ $env == staging ]] && name="File Drawer staging"
  # The unit first, so it reads the file github-oauth.sh writes.
  remote service.sh "ENVIRONMENT=$env"
  if remote_has "$file"; then
    held=1
    keep_secret "GitHub OAuth client secret" "$file" "$stage" && keep=1
  fi
  if (( held && ! keep )); then
    open_url "https://github.com/organizations/filedrawer-dev/settings/applications"
    step "Open $env's app ($name), click 'Generate a new client secret' and copy it."
    step "Once this run has checked it, delete the old secret there."
  elif (( ! held )); then
    if (( ! ASK_ALL )) && [[ "$(_existing "$off_name" || true)" == off ]]; then
      say "GitHub sign-in stays off for $env. To set it up: ops/provision-vps --ask-all $stage"
      SKIPPED+=("$off")
      return 0
    fi
    say "People can sign up and log in with GitHub through an OAuth App of $env's own,"
    say "which asks only for their email addresses. Without one, $env runs with GitHub"
    say "sign-in off, and email sign-up works as before."
    if ! confirm "Set up GitHub sign-in for $env now?"; then
      write_env "$off_name" off
      SKIPPED+=("$off")
      return 0
    fi
    write_env "$off_name" on
    open_url "https://github.com/organizations/filedrawer-dev/settings/applications/new"
    step "Application name: $name"
    step "Homepage URL: $site"
    step "Authorization callback URL: $site/login/github/callback"
    step "That one callback URL, exactly, with wildcard matching off for it: wildcard"
    step "matching would let GitHub send sign-in codes to any subdomain or path below"
    step "it. An app that had a single callback URL before 2026-08-03 has it on; turn"
    step "it off in the app's settings."
    step "Leave Enable Device Flow unchecked, then click Register application."
    step "On the app's page, copy the Client ID, then click 'Generate a new client"
    step "secret' and copy the secret, which GitHub shows only once."
  fi
  # A plain rerun leaves the server's client ID as it is, whatever this checkout
  # saved; under --ask-all, Enter on a held app keeps it too.
  if (( ! held || ASK_ALL )); then
    ask "$id_name" "The client ID:" || printf -v "$id_name" '%s' ""
    id="${!id_name}"
    if [[ -n "$id" ]] || (( ! held )); then
      if [[ ! "$id" =~ ^[A-Za-z0-9.]+$ ]]; then
        warn "That isn't a client ID (letters and digits, as Ov23li...). Rerun to try again."
        exit 1
      fi
      write_env "$id_name" "$id"
    fi
  fi
  CLIENT_SECRET=""
  (( keep )) || ask_new_secret CLIENT_SECRET "The client secret:"
  remote_with_secret github-oauth.sh CLIENT_SECRET "$CLIENT_SECRET" "ENVIRONMENT=$env" "CLIENT_ID=$id"
  unset CLIENT_SECRET
  if (( ! keep )); then
    note "The secret is on the server only; the end of this run checks it with GitHub."
    note "To try it, log in at $site/login with GitHub."
  fi
}
