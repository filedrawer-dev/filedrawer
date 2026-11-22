#!/usr/bin/env bash
# What ops/audit-vps collects from the server, in one ssh round as root: the facts a
# security review checks and the repo can't show. It prints no secret: of an env
# file its key names, of a Caddy site file its owner and mode, of an authorized_keys
# line everything but the key, of a process its user and name.
#
# It changes no file the server keeps. What it leaves are log lines (a PAM session
# for each command run as another user, sudo's own for each `sudo -l`) and ufw's
# lock file in /run.
#
# Each fact is a command's own output under a `== heading ==` line, after a `$` line
# naming the command. A command the server lacks prints `(not found)`, one that
# fails prints `(exit N)` after whatever it said, and the script goes on: it judges
# nothing, and exits 0.
set -uo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"

STAGING_USER=filedrawer
PRODUCTION_USER=filedrawer-prod
UNITS=(
  filedrawer-staging.service filedrawer-production.service caddy.service ssh.service
  filedrawer-backup.service filedrawer-loopback.service
  caddy-upgrade-check.service caddy-upgrade-daily.service
  filedrawer-reboot-check.service filedrawer-reboot-check-daily.service
)
# The root-owned copies of the repo's scripts; ops/audit-vps compares each hash with
# origin/main's.
INSTALLED=(
  /usr/local/sbin/filedrawer-install
  /usr/local/sbin/filedrawer-backup
  /usr/local/sbin/caddy-upgrade-check
  /usr/local/lib/filedrawer-alerts.sh
  /usr/local/sbin/filedrawer-reboot-check
  /usr/local/bin/filedrawer-receive
)

heading() { printf '\n== %s ==\n' "$1"; }

# run CMD ARG...: the command line, then what it printed, then how it failed, if it
# did.
run() {
  printf '$ %s\n' "$*"
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "(not found)"
    return 0
  fi
  local status=0
  "$@" 2>&1 || status=$?
  (( status == 0 )) || echo "(exit $status)"
}

# search LABEL NONE CMD...: like run, for a search that fails when it finds nothing:
# LABEL, then what CMD found, or NONE.
search() {
  local none="$2"
  printf '$ %s\n' "$1"
  shift 2
  "$@" 2>/dev/null || echo "$none"
}

# modes PATH...: owner, group and mode of each path that exists, and which don't.
modes() {
  local path
  for path in "$@"; do
    if [[ -e "$root$path" ]]; then
      stat -c "%U:%G %a $path" "$root$path"
    else
      echo "(absent) $path"
    fi
  done
}

# hashes PATH...: `sha256 PATH HASH` for each file, HASH `none` when it isn't there.
hashes() {
  local path
  for path in "$@"; do
    if [[ -f "$root$path" ]]; then
      echo "sha256 $path $(sha256sum < "$root$path" | cut -d' ' -f1)"
    else
      echo "sha256 $path none"
    fi
  done
}

# as_user USER CMD...: the command as that user. runuser keeps root's environment
# but for HOME, SHELL, USER and LOGNAME.
as_user() {
  local user="$1"
  shift
  runuser -u "$user" -- "$@"
}

# reach USER PORT [PATH]: whether USER can open loopback's PORT, and with PATH, the
# HTTP status a GET of it answers. Only a connection is made to a port with no PATH:
# a request there would be one the server behind it records.
reach() {
  local user="$1" port="$2" path="${3:-}" code
  if ! as_user "$user" true 2>/dev/null; then
    echo "$user -> 127.0.0.1:$port: (can't run as $user)"
  elif ! as_user "$user" timeout 5 bash -c "exec 3<>/dev/tcp/127.0.0.1/$port" 2>/dev/null; then
    echo "$user -> 127.0.0.1:$port: no connection"
  elif [[ -z "$path" ]]; then
    echo "$user -> 127.0.0.1:$port: CONNECTS"
  else
    code=$(as_user "$user" curl -q -s -o /dev/null -w '%{http_code}' --max-time 5 "http://127.0.0.1:$port$path" 2>/dev/null) || true
    echo "$user -> 127.0.0.1:$port: CONNECTS, GET $path answers ${code:-nothing}"
  fi
}

# reach_socket USER SOCKET: whether USER can open Caddy's admin API on the unix
# socket SOCKET, and the HTTP status its GET of the config answers. Only curl's own
# "couldn't connect" (exit 7) counts as no connection.
reach_socket() {
  local user="$1" socket="$2" code status=0
  if [[ ! -S "$root$socket" ]]; then
    echo "$user -> $socket: (no such socket)"
  elif ! as_user "$user" true 2>/dev/null; then
    echo "$user -> $socket: (can't run as $user)"
  else
    code=$(as_user "$user" curl -q -s -o /dev/null -w '%{http_code}' --max-time 5 --unix-socket "$root$socket" http://localhost/config/ 2>/dev/null) || status=$?
    case "$status" in
      0) echo "$user -> $socket: CONNECTS, GET /config/ answers $code" ;;
      7) echo "$user -> $socket: no connection" ;;
      *) echo "$user -> $socket: (the probe failed: curl exit $status)" ;;
    esac
  fi
}

heading "system"
run cat "$root/etc/debian_version"
run uname -r
run uptime
# Debian writes this file for kernel packages only: `no` says nothing of a library
# upgrade whose users haven't restarted.
if [[ -e "$root/var/run/reboot-required" ]]; then
  echo "reboot-required: yes"
  run cat "$root/var/run/reboot-required.pkgs"
else
  echo "reboot-required: no"
fi

heading "listening sockets"
run ss -H -ltnp
run ss -H -lunp
run ss -H -lxp

heading "firewall"
run ufw status verbose
run nft list ruleset
run systemctl is-enabled filedrawer-loopback.service
# Enabled isn't loaded: a load that failed at boot leaves the ports open.
run systemctl is-active filedrawer-loopback.service

heading "loopback reach, by user"
# Caddy's admin API, on its default port and on the socket ops/remote/caddy.sh
# gives it, production's port and staging's: which user reaches which. Root reaches
# the socket, which shows the probe itself works.
admin_socket=/var/lib/caddy/admin.sock
reach_socket root "$admin_socket"
for user in "$STAGING_USER" "$PRODUCTION_USER" deploy nobody; do
  reach "$user" 2019 /config/
  reach_socket "$user" "$admin_socket"
  reach "$user" 8080
  reach "$user" 8081
done
# Caddy must still reach both apps, or the sites are down behind it.
reach caddy 8080
reach caddy 8081

heading "sshd"
run sshd -T
# What a Match block would change for each user who logs in.
for user in root deploy; do
  run sshd -T -C "user=$user,host=audit.invalid,addr=203.0.113.1"
done
modes /etc/ssh/sshd_config /etc/ssh/sshd_config.d /etc/ssh/sshd_config.d/10-filedrawer.conf

heading "authorized_keys"
# Each line with its key replaced, so the options and the key's type show.
for home in /root /home/deploy; do
  file="$home/.ssh/authorized_keys"
  modes "$home" "$home/.ssh" "$file"
  if [[ -r "$root$file" ]]; then
    sed -E 's#(ssh-[a-z0-9@.-]+|ecdsa-[a-z0-9@.-]+|sk-[a-z0-9@.-]+)[ \t]+[A-Za-z0-9+/=]+#\1 <key>#' "$root$file"
  fi
done
run find "$root/home" "$root/root" "$root/srv/filedrawer" -maxdepth 3 -name 'authorized_keys*'

heading "users"
printf '$ accounts with a login shell\n'
grep -Ev ':(/usr/sbin/nologin|/sbin/nologin|/bin/false)$' "$root/etc/passwd" 2>/dev/null |
  cut -d: -f1,3,4,6,7 || echo "(unreadable)"
for user in root deploy "$STAGING_USER" "$PRODUCTION_USER" caddy; do
  run passwd -S "$user"
  run id "$user"
done

heading "sudo"
run visudo -c
search "rules in /etc/sudoers and /etc/sudoers.d (comments left out)" "(none read)" \
  grep -rEv '^\s*(#|$)' "$root/etc/sudoers" "$root/etc/sudoers.d"
modes /etc/sudoers /etc/sudoers.d
run find "$root/etc/sudoers.d" -type f -exec stat -c '%U:%G %a %n' {} +
run sudo -l -U deploy
run sudo -l -U "$STAGING_USER"
run sudo -l -U "$PRODUCTION_USER"

heading "installed scripts"
hashes "${INSTALLED[@]}"
modes "${INSTALLED[@]}" /usr/local/sbin /usr/local/bin

heading "installed units and configs: hashes, owners and modes"
# The files the repo's scripts generate that hold no secret.
generated=(/etc/caddy/Caddyfile /etc/filedrawer/loopback.nft
  /etc/ssh/sshd_config.d/10-filedrawer.conf /etc/apt/apt.conf.d/52filedrawer-caddy
  /etc/apt/apt.conf.d/53filedrawer-reboot)
for file in "$root"/etc/systemd/system/filedrawer-* "$root"/etc/systemd/system/caddy-upgrade-* \
  "$root"/etc/sudoers.d/*; do
  [[ -f "$file" ]] && generated+=("${file#"$root"}")
done
hashes "${generated[@]}"
modes "${generated[@]}"

heading "secrets: owners, modes and key names, never values"
modes /etc/filedrawer
for file in "$root"/etc/filedrawer/*; do
  [[ -e "$file" ]] || continue
  name="${file#"$root"}"
  modes "$name"
  case "$name" in
    *.env)
      echo "  lines: $(grep -c '' "$file")"
      echo "  keys: $(sed -n 's/^\(export \)\{0,1\}\([A-Za-z_][A-Za-z0-9_]*\)=.*/\2/p' "$file" | paste -sd' ' -)"
      ;;
  esac
done

heading "caddy"
run caddy version
run apt-mark showhold
modes /etc/caddy /etc/caddy/Caddyfile /etc/caddy/sites /etc/caddy/field-test.caddy
for file in "$root"/etc/caddy/sites/*; do
  [[ -e "$file" ]] && modes "${file#"$root"}"
done
# The Caddyfile's global options and imports. A site's lines hold the staging key
# and a password's hash, so a line shows only when it is a brace alone, or `admin`,
# `servers` or `import` with at most one word after it that starts no hash (`$`).
search "/etc/caddy/Caddyfile: admin, servers, import and brace lines" "(none)" \
  grep -nE '^\s*((admin|servers|import)(\s+[^$[:space:]]\S*)?\s*\{?|[{}])\s*$' "$root/etc/caddy/Caddyfile"
run ls -la "$root/var/lib/filedrawer/caddy-guard"
run ls -la "$root/var/log/caddy"
search "Caddy logs that hold a ?key= (names only)" "(none)" grep -rl 'key=' "$root/var/log/caddy"

heading "data and binaries"
modes /srv/filedrawer \
  /srv/filedrawer/staging /srv/filedrawer/staging/bin /srv/filedrawer/staging/bin/filedrawer \
  /srv/filedrawer/staging/data /srv/filedrawer/staging/data/filedrawer.db \
  /srv/filedrawer/production /srv/filedrawer/production/bin \
  /srv/filedrawer/production/bin/filedrawer /srv/filedrawer/production/data \
  /srv/filedrawer/production/data/filedrawer.db /srv/filedrawer/production/data/backups \
  /var/lib/filedrawer-deploy /var/lib/filedrawer
printf '$ each service user reading the other environment\n'
for pair in "$STAGING_USER:production" "$PRODUCTION_USER:staging"; do
  user="${pair%%:*}" other="${pair##*:}"
  for path in "/srv/filedrawer/$other/data/filedrawer.db" "/srv/filedrawer/$other/data" "/etc/filedrawer/$other.env"; do
    if ! as_user "$user" true 2>/dev/null; then
      echo "$user reading $path: (can't run as $user)"
    elif as_user "$user" test -r "$root$path" 2>/dev/null; then
      echo "$user CAN read $path"
    else
      echo "$user can't read $path"
    fi
  done
done
run ls -la "$root/srv/filedrawer/production/data/backups"

heading "units"
run systemctl list-units --failed --no-pager --plain
run systemctl list-timers --all --no-pager --plain
run systemctl is-enabled "${UNITS[@]}" unattended-upgrades.service
for unit in "${UNITS[@]}"; do
  # No Environment property: a unit's settings may hold a secret.
  run systemctl show "$unit" -p User -p Group -p NoNewPrivileges -p ProtectSystem \
    -p ProtectHome -p PrivateTmp -p PrivateDevices -p CapabilityBoundingSet \
    -p AmbientCapabilities -p RestrictAddressFamilies -p SystemCallFilter \
    -p ReadWritePaths -p InaccessiblePaths -p MemoryDenyWriteExecute \
    -p IPAddressAllow -p IPAddressDeny -p SocketBindAllow -p SocketBindDeny \
    -p EnvironmentFiles -p ExecStart -p ExecReload -p DropInPaths
  run systemd-analyze security --no-pager "$unit"
done
run find "$root/etc/systemd/system" -name '*.d' -type d

heading "updates"
# A simulation, with apt's package caches kept in memory, so it rewrites neither.
run apt-get -s -o Debug::NoLocking=true -o Dir::Cache::pkgcache= -o Dir::Cache::srcpkgcache= upgrade
run stat -c '%y %n' "$root/var/lib/apt/periodic/update-stamp" "$root/var/lib/apt/periodic/upgrade-stamp"
run tail -n 40 "$root/var/log/unattended-upgrades/unattended-upgrades.log"
run find "$root/etc/apt/sources.list" "$root/etc/apt/sources.list.d" -type f

heading "processes: user and name"
run ps -eo user:20,comm --no-headers --sort=user

heading "journal"
run journalctl --disk-usage
run systemd-analyze cat-config systemd/journald.conf

exit 0
