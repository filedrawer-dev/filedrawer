#!/usr/bin/env bash
# The server reboots itself for a kernel it installed, with notice (design.md,
# Hosting and operations). Writes:
#   filedrawer-reboot-check.service, which runs /usr/local/sbin/filedrawer-reboot-check
#     (the wizard installs it) at each boot and after each dpkg run, and never reboots;
#   filedrawer-reboot-check-daily.service and its timer, at 05:00 UTC, the one run
#     that may reboot;
#   /etc/apt/apt.conf.d/53filedrawer-reboot, which starts the first after each dpkg
#     run, so a new kernel is mailed about within minutes.
# It installs libpam-systemd when missing: without it sshd registers no session, and
# the check that nobody is logged in passes unseen. Then it runs the check once,
# which notices a reboot already pending. The check mails through the Caddy guard's
# settings, which caddy-upgrades.sh writes first. Runs as root on Debian; safe to
# rerun.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"

if [[ ! -f "$root/etc/filedrawer/caddy-upgrades.env" ]]; then
  echo "the reboot check mails through the Caddy guard's settings, and the server has none; run ops/provision-vps caddy-upgrades first" >&2
  exit 1
fi

if [[ "$(dpkg-query -W -f='${Status}' libpam-systemd 2>/dev/null)" != "install ok installed" ]]; then
  DEBIAN_FRONTEND=noninteractive apt-get -y -q install libpam-systemd
fi

mkdir -p "$root/etc/apt/apt.conf.d" "$root/etc/systemd/system"
cat > "$root/etc/apt/apt.conf.d/53filedrawer-reboot" <<'CONF'
// Written by ops/remote/reboots.sh; edits here are lost on the next run.
// After each dpkg run, the reboot check notices a kernel that needs a reboot and
// mails the owner; it never reboots from here. A failure here must never fail apt.
DPkg::Post-Invoke {
	"[ -d /run/systemd/system ] && systemctl start --no-block filedrawer-reboot-check.service >/dev/null 2>&1 || true";
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
ExecStart=/usr/local/sbin/filedrawer-reboot-check$2
# Its lock waits up to 10 minutes; the daily run tries until 05:25, and after
# asking for the reboot waits 10 minutes for it.
TimeoutStartSec=50min
CONF
}
{
  unit "notice a pending reboot, and report after one this job made" ""
  printf '\n[Install]\nWantedBy=multi-user.target\n'
} > "$root/etc/systemd/system/filedrawer-reboot-check.service"
unit "reboot for a pending kernel, at 05:00 UTC once it is due" " --daily" \
  > "$root/etc/systemd/system/filedrawer-reboot-check-daily.service"
cat > "$root/etc/systemd/system/filedrawer-reboot-check-daily.timer" <<'CONF'
[Unit]
Description=File Drawer: the reboot check's daily run, at 05:00 UTC

[Timer]
OnCalendar=*-*-* 05:00:00 UTC
AccuracySec=1s
# Not Persistent: a 05:00 missed while the server was down must not run at boot.
# The script also refuses to reboot outside 05:00 to 05:30 UTC.

[Install]
WantedBy=timers.target
CONF

systemctl daemon-reload
systemctl enable filedrawer-reboot-check.service
systemctl enable --now filedrawer-reboot-check-daily.timer
systemctl start filedrawer-reboot-check.service ||
  echo "the reboot check's first run found something to report; its mail, or journalctl -u filedrawer-reboot-check, says what" >&2

echo "reboots: done"
