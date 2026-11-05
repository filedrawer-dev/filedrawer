#!/usr/bin/env bash
# The systemd unit for staging. It stays idle until a deploy puts the binary at
# /srv/filedrawer/staging/bin/filedrawer. Runs as root; safe to rerun.
set -euo pipefail

cat > /etc/filedrawer/staging.env <<'CONF'
FILEDRAWER_DATABASE=/srv/filedrawer/staging/data/filedrawer.db
FILEDRAWER_LISTEN=127.0.0.1:8081
FILEDRAWER_ENVIRONMENT=staging
FILEDRAWER_SCREENER=pass-all
RUST_LOG=info
CONF
chown root:filedrawer /etc/filedrawer/staging.env
chmod 0640 /etc/filedrawer/staging.env

cat > /etc/systemd/system/filedrawer-staging.service <<'CONF'
[Unit]
Description=File Drawer (staging)
After=network-online.target
Wants=network-online.target
ConditionPathIsExecutable=/srv/filedrawer/staging/bin/filedrawer

[Service]
User=filedrawer
Group=filedrawer
EnvironmentFile=/etc/filedrawer/staging.env
ExecStart=/srv/filedrawer/staging/bin/filedrawer serve
Restart=on-failure
RestartSec=2
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ReadWritePaths=/srv/filedrawer/staging/data
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictSUIDSGID=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
CapabilityBoundingSet=

[Install]
WantedBy=multi-user.target
CONF
systemctl daemon-reload
systemctl enable filedrawer-staging.service

echo "staging service: done (it starts after the first deploy)"
