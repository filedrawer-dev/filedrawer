#!/usr/bin/env bash
# Caddy follows its releases, with a guard (#100; design.md, Hosting and
# operations). Writes:
#   /etc/apt/apt.conf.d/52filedrawer-caddy  unattended-upgrades takes Caddy's apt
#     repository too, keeps an edited conffile (caddy.sh rewrites the packaged
#     Caddyfile) rather than skip the upgrade, after each dpkg run starts
#     caddy-upgrade-check.service, which waits for nothing, and marks a failed
#     signature on any repository for the guard's daily run (#110);
#   caddy-upgrade-check.service, caddy-upgrade-daily.service and its timer, which run
#     /usr/local/sbin/caddy-upgrade-check (the wizard installs it);
#   /etc/filedrawer/caddy-upgrades.env, from "<ALERT_TO><TAB><PING>" as the first
#     line of stdin, into $SETTINGS: the address the guard mails and its
#     healthchecks.io ping URL, a secret, so neither sits in a command line. An empty
#     address or ping URL keeps the one stored (#112), and an empty line keeps the
#     whole file.
# Then starts the guard once, which records the running Caddy as good if it passes.
# Leaves a hold alone: a hold the guard set waits for the owner to clear it.
# Runs as root on Debian; safe to rerun.
set -euo pipefail
: "${SETTINGS?pass '<ALERT_TO><TAB><PING>' as the first line of stdin, or an empty line}"

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
env_file="$root/etc/filedrawer/caddy-upgrades.env"

if [[ -n "$SETTINGS" ]]; then
  if [[ "$SETTINGS" != *$'\t'* ]]; then
    echo "expected '<ALERT_TO><TAB><PING>'" >&2
    exit 1
  fi
  alert_to="${SETTINGS%%$'\t'*}"
  ping="${SETTINGS#*$'\t'}"
  unset SETTINGS
  [[ -n "$ping" ]] || ping=$(sed -n 's/^PING=//p' "$env_file" 2>/dev/null || true)
  [[ -n "$alert_to" ]] || alert_to=$(sed -n 's/^ALERT_TO=//p' "$env_file" 2>/dev/null || true)
  if [[ ! "$alert_to" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]+$ ]]; then
    echo "the address isn't one address, such as ops@filedrawer.dev" >&2
    exit 1
  fi
  # A secret, so a refusal never repeats it.
  if [[ ! "$ping" =~ ^https://hc-ping\.com/[A-Za-z0-9_./-]+$ ]]; then
    echo "the ping isn't a healthchecks.io ping URL, such as https://hc-ping.com/<uuid>; to give one, run ops/provision-vps --ask-all caddy-upgrades" >&2
    exit 1
  fi
  mkdir -p "$root/etc/filedrawer"
  (
    umask 077
    printf 'ALERT_TO=%s\nPING=%s\n' "$alert_to" "$ping" > "$env_file"
  )
  chown root:root "$env_file"
  chmod 0600 "$env_file"
  unset ping
elif [[ ! -f "$env_file" ]]; then
  echo "no settings given and none on the server; run ops/provision-vps --ask-all caddy-upgrades" >&2
  exit 1
fi

mkdir -p "$root/etc/apt/apt.conf.d" "$root/etc/systemd/system"
cat > "$root/etc/apt/apt.conf.d/52filedrawer-caddy" <<'CONF'
// Written by ops/remote/caddy-upgrades.sh (#100); edits here are lost on the next run.
// Caddy keeps only its latest release and ships security fixes beside breaking
// changes, so the server follows its releases, and caddy-upgrade-check rolls back
// one whose checks fail.
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
Unattended-Upgrade::Origins-Pattern {
	"origin=cloudsmith/caddy/stable";
};
// Without these, unattended-upgrades skips a Caddy release whose packaged
// Caddyfile changed, since caddy.sh rewrote it and dpkg would ask.
Dpkg::Options {
	"--force-confdef";
	"--force-confold";
};
// --no-block: the guard's rollback runs apt, which would wait forever on the lock
// this dpkg run holds. A failure here must never fail apt.
DPkg::Post-Invoke {
	"[ -d /run/systemd/system ] && systemctl start --no-block caddy-upgrade-check.service >/dev/null 2>&1 || true";
};
// When a repository's signature fails (an expired or replaced key) but its old
// lists are on disk, apt keeps them, counts the failure as transient, runs these,
// and still exits 0, so apt's daily update-stamp moves while that repository's
// upgrades stop. Each update clears the marker first, so it stands for the last
// update only; caddy-upgrade-check --daily reads it. A failing hook would fail
// apt-get update.
APT::Update::Pre-Invoke {
	"rm -f /var/lib/filedrawer/caddy-guard/auth-failure || true";
};
APT::Update::Auth-Failure {
	"mkdir -p /var/lib/filedrawer/caddy-guard && touch /var/lib/filedrawer/caddy-guard/auth-failure || true";
};
CONF

unit() {
  cat <<CONF
[Unit]
Description=File Drawer: $1
After=network-online.target caddy.service filedrawer-staging.service filedrawer-production.service
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/caddy-upgrade-check$2
# A run that hangs would hold the lock every later run waits on; past flock's 30
# minutes, and a rollback's two installs of up to 15 each.
TimeoutStartSec=90min
CONF
}
unit "check Caddy after a dpkg run, and roll back an upgrade that breaks the sites" "" \
  > "$root/etc/systemd/system/caddy-upgrade-check.service"
unit "check Caddy daily, and ping healthchecks.io" " --daily" \
  > "$root/etc/systemd/system/caddy-upgrade-daily.service"
cat > "$root/etc/systemd/system/caddy-upgrade-daily.timer" <<'CONF'
[Unit]
Description=File Drawer: check Caddy daily, after unattended-upgrades' morning run

[Timer]
# unattended-upgrades runs at 06:00 plus up to an hour.
OnCalendar=*-*-* 08:00:00
Persistent=true

[Install]
WantedBy=timers.target
CONF

systemctl daemon-reload
systemctl enable --now caddy-upgrade-daily.timer
systemctl start caddy-upgrade-check.service ||
  echo "the guard's first check failed; its mail, or journalctl -u caddy-upgrade-check, says why" >&2

echo "caddy upgrades: done"
