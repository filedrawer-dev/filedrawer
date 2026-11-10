#!/usr/bin/env bash
# The systemd unit and base settings for ENVIRONMENT, staging or production. The unit
# stays idle until a deploy puts the binary at /srv/filedrawer/<env>/bin/filedrawer.
# Each runs as its own user (service-user.sh) and may bind only its own port, so a
# binary sent with staging's deploy key can't take production's while production is
# down. Production's unit also needs loopback.sh's rules loaded, which keep staging's
# user from connecting to production's port. Settings files are root's alone: systemd
# reads them as root. Runs as root; safe to rerun. Exits 2 for an environment it
# doesn't know.
set -euo pipefail

case "${ENVIRONMENT-}" in
  staging) user=filedrawer port=8081 host=staging.filedrawer.dev ;;
  production) user=filedrawer-prod port=8080 host=filedrawer.dev ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
env="$ENVIRONMENT"
home="/srv/filedrawer/$env"

mkdir -p "$root/etc/filedrawer" "$root/etc/systemd/system"
settings="$root/etc/filedrawer/$env.env"
umask 077
{
  printf 'FILEDRAWER_DATABASE=%s/data/filedrawer.db\n' "$home"
  printf 'FILEDRAWER_LISTEN=127.0.0.1:%s\n' "$port"
  printf 'FILEDRAWER_ENVIRONMENT=%s\n' "$env"
  # Staging publishes unscreened until screening.sh's file replaces this; production
  # refuses pass-all, so it starts only once screening.sh has run.
  [[ $env == staging ]] && printf 'FILEDRAWER_SCREENER=pass-all\n'
  printf 'FILEDRAWER_PUBLIC_URL=https://%s\n' "$host"
  printf 'FILEDRAWER_FILES_URL=https://files.%s\n' "$host"
  printf 'RUST_LOG=info\n'
} > "$settings"
chown root:root "$settings"
chmod 0600 "$settings"
umask 022

if [[ $env == staging ]]; then
  needs=""
  extra_settings="# The smoke-test account, which ops/remote/smoke-env.sh names: its entries aren't
# screened.
EnvironmentFile=-/etc/filedrawer/staging-smoke.env"
  # A second layer beside production's folder modes.
  extra_sandbox="InaccessiblePaths=-/srv/filedrawer/production"
else
  needs="Requires=filedrawer-loopback.service
After=filedrawer-loopback.service"
  extra_settings="# The healthchecks.io URL the daily email pings, which ops/remote/ping.sh writes;
# production refuses to start without it.
EnvironmentFile=-/etc/filedrawer/production-ping.env"
  extra_sandbox="InaccessiblePaths=-/srv/filedrawer/staging"
fi

cat > "$root/etc/systemd/system/filedrawer-$env.service" <<CONF
[Unit]
Description=File Drawer ($env)
After=network-online.target
Wants=network-online.target
${needs:+$needs
}ConditionPathIsExecutable=$home/bin/filedrawer

[Service]
User=$user
Group=$user
EnvironmentFile=/etc/filedrawer/$env.env
# The SMTP settings, which ops/remote/mail.sh writes; without them email goes to the
# log, and production refuses to start.
EnvironmentFile=-/etc/filedrawer/$env-mail.env
# The screening settings, which ops/remote/screening.sh writes. Read after $env.env,
# so its FILEDRAWER_SCREENER replaces staging's pass-all.
EnvironmentFile=-/etc/filedrawer/$env-screening.env
$extra_settings
# The GitHub token for the link checks, which ops/remote/github-token.sh writes, one
# file for both environments. Without it, 60 API calls an hour.
EnvironmentFile=-/etc/filedrawer/github.env
ExecStart=$home/bin/filedrawer serve
Restart=on-failure
RestartSec=2
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ReadWritePaths=$home/data
$extra_sandbox
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictSUIDSGID=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
SocketBindAllow=tcp:$port
SocketBindDeny=any
CapabilityBoundingSet=

[Install]
WantedBy=multi-user.target
CONF
systemctl daemon-reload
systemctl enable "filedrawer-$env.service"

echo "$env service: done (it starts after the first deploy)"
