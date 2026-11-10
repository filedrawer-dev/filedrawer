#!/usr/bin/env bash
# Production's nightly copy during the beta (#41; design.md, Hosting and operations):
# backups/ beside production's database, and filedrawer-backup.service with its
# timer, which runs /usr/local/sbin/filedrawer-backup (the wizard installs it) at
# 23:30 UTC as filedrawer-prod, so the daily email just after midnight reports a
# fresh copy. Staging has no backups. Needs service-user.sh's user, service.sh's
# production.env and ping.sh's production-backup.env. Runs as root; safe to rerun.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
home=/srv/filedrawer/production
user=filedrawer-prod

mkdir -p "$root$home/data/backups" "$root/etc/systemd/system"
chown "$user:$user" "$root$home/data/backups"
chmod 0700 "$root$home/data/backups"

cat > "$root/etc/systemd/system/filedrawer-backup.service" <<CONF
[Unit]
Description=File Drawer: production's nightly copy of its database
# Until the first deploy there's nothing to copy, and the job skips without pinging.
ConditionPathExists=$home/data/filedrawer.db

[Service]
Type=oneshot
User=$user
Group=$user
# FILEDRAWER_DATABASE, as the server reads it.
EnvironmentFile=/etc/filedrawer/production.env
# BACKUP_PING, which ops/remote/ping.sh writes. Required: a job that can't report
# would fail unseen.
EnvironmentFile=/etc/filedrawer/production-backup.env
ExecStart=/usr/local/sbin/filedrawer-backup
# A run that hangs would stop every later night's.
TimeoutStartSec=1h
Nice=10
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
# A WAL reader writes -shm beside the database.
ReadWritePaths=$home/data
InaccessiblePaths=-/srv/filedrawer/staging
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictSUIDSGID=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
SocketBindDeny=any
CapabilityBoundingSet=
CONF

cat > "$root/etc/systemd/system/filedrawer-backup.timer" <<'CONF'
[Unit]
Description=File Drawer: production's nightly copy, at 23:30 UTC

[Timer]
OnCalendar=*-*-* 23:30:00 UTC
# A night missed while the server was down runs at boot.
Persistent=true

[Install]
WantedBy=timers.target
CONF
systemctl daemon-reload
systemctl enable --now filedrawer-backup.timer

echo "nightly copy: done (it skips until the first deploy)"
