#!/usr/bin/env bash
# The server's memory check (#64; design.md, Hosting and operations). Reads its
# healthchecks.io ping URL as the first line of stdin, into $MEMORY_PING, since
# anyone holding it can report the server healthy; an empty line keeps the one
# stored (#112). Writes it to /etc/filedrawer/memory.env, root's alone, and writes
#   filedrawer-memory-check.service, which runs /usr/local/sbin/filedrawer-memory-check
#     (the wizard installs it), and
#   filedrawer-memory-check.timer, every 5 minutes,
# then runs the check once, so its healthchecks.io check goes up at once. The check
# serves the one server, staging and production alike. Runs as root on Debian; safe
# to rerun.
set -euo pipefail
: "${MEMORY_PING?pass the ping URL of the memory check as the first line of stdin}"

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
settings="$root/etc/filedrawer/memory.env"
url="$MEMORY_PING"
unset MEMORY_PING
[[ -n "$url" ]] || url=$(sed -n 's/^MEMORY_PING=//p' "$settings" 2>/dev/null || true)
# The URL is a secret, so a refusal never shows it.
if [[ ! "$url" =~ ^https://hc-ping\.com/[A-Za-z0-9_./-]+$ ]]; then
  echo "the memory check's ping isn't a healthchecks.io ping URL, such as https://hc-ping.com/<uuid>; to give one, run ops/provision-vps --ask-all memory" >&2
  exit 1
fi

mkdir -p "$root/etc/filedrawer" "$root/etc/systemd/system"
(
  umask 077
  printf 'MEMORY_PING=%s\n' "$url" > "$settings.new"
)
chown root:root "$settings.new"
chmod 0600 "$settings.new"
mv -f "$settings.new" "$settings"

cat > "$root/etc/systemd/system/filedrawer-memory-check.service" <<'CONF'
[Unit]
Description=File Drawer: ping healthchecks.io, as failed when processes use more than half the RAM or swap out pages
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
# MEMORY_PING, which ops/remote/memory.sh writes. Required: a check that can't
# report would fail unseen.
EnvironmentFile=/etc/filedrawer/memory.env
ExecStart=/usr/local/sbin/filedrawer-memory-check
# Its ping retries for about a minute; the next run is 5 minutes off.
TimeoutStartSec=4min
DynamicUser=yes
# The last run's pswpout, to tell pages swapped out since from swap merely held.
StateDirectory=filedrawer-memory-check
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
InaccessiblePaths=-/srv/filedrawer
# Hides other users' processes; /proc/meminfo stays. ProcSubset=pid would hide it.
ProtectProc=invisible
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictSUIDSGID=yes
RestrictNamespaces=yes
RestrictRealtime=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
SocketBindDeny=any
CapabilityBoundingSet=
SystemCallArchitectures=native
CONF

cat > "$root/etc/systemd/system/filedrawer-memory-check.timer" <<'CONF'
[Unit]
Description=File Drawer: the memory check, every 5 minutes

[Timer]
OnCalendar=*:0/5

[Install]
WantedBy=timers.target
CONF

systemctl daemon-reload
systemctl enable --now filedrawer-memory-check.timer
systemctl start filedrawer-memory-check.service ||
  echo "the memory check's first run failed; journalctl -u filedrawer-memory-check says why" >&2

echo "memory check: done"
