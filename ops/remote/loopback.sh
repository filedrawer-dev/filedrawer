#!/usr/bin/env bash
# Keeps staging's user off production's port (design.md, Hosting and operations).
# Production listens on 127.0.0.1:8080 and trusts X-Forwarded-For from loopback, so a
# binary sent with staging's deploy key could otherwise connect there and pose as any
# client. A table of its own, which `nft -f` replaces whole on each load, rejects
# every connection from the user `filedrawer` to port 8080 on loopback; ufw's rules,
# in tables of their own, stay as they are. (/etc/nftables.conf isn't used: Debian's
# starts with `flush ruleset`, which would drop ufw's.) A oneshot unit loads the rules
# at boot, before production starts. The units' SocketBindAllow keeps staging from
# binding the port. Runs as root; safe to rerun.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
rules="$root/etc/filedrawer/loopback.nft"

if ! command -v nft >/dev/null 2>&1; then
  DEBIAN_FRONTEND=noninteractive apt-get -y -q install nftables
fi

mkdir -p "$root/etc/filedrawer" "$root/etc/systemd/system"
tmp=$(mktemp "$root/etc/filedrawer/.loopback.XXXXXX")
trap 'rm -f "$tmp"' EXIT
cat > "$tmp" <<'NFT'
# Written by ops/remote/loopback.sh; edits here are lost on the next run.
# The empty table first, so the delete never fails, then the table anew.
table inet filedrawer_loopback
delete table inet filedrawer_loopback
table inet filedrawer_loopback {
	chain output {
		type filter hook output priority 0; policy accept;
		meta skuid "filedrawer" ip daddr 127.0.0.0/8 tcp dport 8080 reject with tcp reset
		meta skuid "filedrawer" ip6 daddr ::1 tcp dport 8080 reject with tcp reset
	}
}
NFT
nft -c -f "$tmp" || { echo "nft refused the loopback rules; nothing changed" >&2; exit 1; }
chmod 0644 "$tmp"
mv "$tmp" "$rules"

cat > "$root/etc/systemd/system/filedrawer-loopback.service" <<'UNIT'
[Unit]
Description=File Drawer: staging's user can't reach production's port
Before=filedrawer-production.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/sbin/nft -f /etc/filedrawer/loopback.nft

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable filedrawer-loopback.service
systemctl restart filedrawer-loopback.service

echo "loopback: done"
