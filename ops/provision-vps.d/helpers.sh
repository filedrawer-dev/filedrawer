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
