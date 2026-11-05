#!/usr/bin/env bash
# Base system for the File Drawer VPS (design.md, Hosting and operations): updates,
# automatic security updates, the firewall, a small swapfile, journald capped at
# 200 MB and 14 days, and SSH by key only. Runs as root on Debian; safe to rerun.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

apt-get update -q
apt-get -y -q upgrade
apt-get -y -q install unattended-upgrades ufw curl ca-certificates gnupg \
  debian-keyring debian-archive-keyring apt-transport-https sqlite3

# Only SSH and the web; HTTP/3 needs UDP 443.
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw allow 443/udp
ufw --force enable

# A guard against the OOM killer, never part of the memory plan.
if ! swapon --show=NAME --noheadings | grep -qx /swapfile; then
  fallocate -l 1G /swapfile
  chmod 600 /swapfile
  mkswap /swapfile
  swapon /swapfile
fi
grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab

# 14 days keeps the privacy page's promise on IP addresses in logs.
mkdir -p /etc/systemd/journald.conf.d
cat > /etc/systemd/journald.conf.d/filedrawer.conf <<'CONF'
[Journal]
SystemMaxUse=200M
MaxRetentionSec=14d
CONF
systemctl restart systemd-journald

cat > /etc/ssh/sshd_config.d/10-filedrawer.conf <<'CONF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
CONF
sshd -t
systemctl reload ssh

echo "base: done"
