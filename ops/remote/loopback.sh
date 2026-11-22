#!/usr/bin/env bash
# Lets only Caddy and root open the apps' ports and Caddy's admin port on this
# machine (design.md, Hosting and operations; security review, LIVE-3). Production
# listens on 127.0.0.1:8080 and staging on 8081, both trust X-Forwarded-For from
# loopback, and staging's Caddy, not the app, asks for its key: any other local
# user, staging's and production's service users, `deploy` and `nobody` among them,
# could otherwise reach an app directly and pose as any client, or reach staging
# without its key. Port 2019 is Caddy's default admin API, which ops/remote/caddy.sh
# moves to a unix socket; the rule covers it should a hand-edited Caddyfile bring
# the default back. It matches any local address, not loopback alone, since a
# listener on the public address is just as near.
#
# Only new connections are matched: a packet of an open one, or of a socket in
# TIME_WAIT, which has no user to match, passes. A table of its own, which `nft -f`
# replaces whole on each load, holds the rules; ufw's, in tables of their own, stay
# as they are. (/etc/nftables.conf isn't used: Debian's starts with `flush ruleset`,
# which would drop ufw's.) A oneshot unit loads them at boot, before either app
# starts. The units' SocketBindAllow keeps each app to its own port.
#
# After loading, for each port an app listens on, it checks that Caddy still
# connects and `nobody` can't; if Caddy is cut off it puts the previous rules back.
# Runs as root, after Caddy is installed; safe to rerun.
set -euo pipefail

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
rules="$root/etc/filedrawer/loopback.nft"
ports=(8080 8081)

if ! id caddy >/dev/null 2>&1; then
  echo "no user caddy: install Caddy first (staging's 'Caddy and basic auth' stage); nothing changed" >&2
  exit 1
fi
if ! command -v nft >/dev/null 2>&1; then
  DEBIAN_FRONTEND=noninteractive apt-get -y -q install nftables
fi

mkdir -p "$root/etc/filedrawer" "$root/etc/systemd/system"
tmp=$(mktemp "$root/etc/filedrawer/.loopback.XXXXXX")
previous=$(mktemp "$root/etc/filedrawer/.loopback-previous.XXXXXX")
trap 'rm -f "$tmp" "$previous"' EXIT
cat > "$tmp" <<'NFT'
# Written by ops/remote/loopback.sh; edits here are lost on the next run.
# The empty table first, so the delete never fails, then the table anew.
table inet filedrawer_loopback
delete table inet filedrawer_loopback
table inet filedrawer_loopback {
	chain output {
		type filter hook output priority 0; policy accept;
		ct state new fib daddr type local tcp dport { 8080, 8081, 2019 } meta skuid "root" accept
		ct state new fib daddr type local tcp dport { 8080, 8081, 2019 } meta skuid "caddy" accept
		ct state new fib daddr type local tcp dport { 8080, 8081, 2019 } reject with tcp reset
	}
}
NFT
nft -c -f "$tmp" || { echo "nft refused the loopback rules; nothing changed" >&2; exit 1; }
if [[ -f "$rules" ]]; then
  cp "$rules" "$previous"
else
  # No rules before: putting them back means an empty table.
  printf 'table inet filedrawer_loopback\ndelete table inet filedrawer_loopback\n' > "$previous"
fi
chmod 0644 "$tmp"
mv "$tmp" "$rules"

cat > "$root/etc/systemd/system/filedrawer-loopback.service" <<'UNIT'
[Unit]
Description=File Drawer: only Caddy and root reach the apps' ports
Before=filedrawer-production.service filedrawer-staging.service

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

# connects USER PORT: whether USER can open 127.0.0.1:PORT.
connects() {
  runuser -u "$1" -- timeout 5 bash -c "exec 3<>/dev/tcp/127.0.0.1/$2" 2>/dev/null
}
for port in "${ports[@]}"; do
  [[ -n "$(ss -Hltn "sport = :$port")" ]] || continue
  if ! connects caddy "$port"; then
    echo "caddy can't reach 127.0.0.1:$port under the new rules, so the site behind it would be down; putting the previous rules back" >&2
    cp "$previous" "$rules"
    chmod 0644 "$rules"
    systemctl restart filedrawer-loopback.service
    exit 1
  fi
  if connects nobody "$port"; then
    echo "nobody reaches 127.0.0.1:$port: the rules aren't taking effect; check 'nft list ruleset'" >&2
    exit 1
  fi
done

echo "loopback: done"
