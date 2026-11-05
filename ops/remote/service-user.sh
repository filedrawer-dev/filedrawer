#!/usr/bin/env bash
# The account the server runs as, and its directories. Runs as root; safe to rerun.
set -euo pipefail

if ! id filedrawer >/dev/null 2>&1; then
  useradd --system --home-dir /srv/filedrawer --shell /usr/sbin/nologin filedrawer
fi
install -d -o root -g root -m 0755 /srv/filedrawer /srv/filedrawer/staging /srv/filedrawer/staging/bin
install -d -o filedrawer -g filedrawer -m 0750 /srv/filedrawer/staging/data
install -d -o root -g filedrawer -m 0750 /etc/filedrawer

echo "service user: done"
