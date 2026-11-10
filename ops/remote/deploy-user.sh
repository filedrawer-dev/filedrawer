#!/usr/bin/env bash
# The `deploy` user GitHub Actions logs in as (design.md, Hosting and operations).
# Its one key may only run filedrawer-receive, which hands an upload to
# filedrawer-install through the one sudo rule below. DEPLOY_PUBLIC_KEY is the key's
# public half; the wizard has already copied both scripts into place. An empty
# DEPLOY_PUBLIC_KEY keeps the key the server has and rewrites the rest, the sudo rule
# included, as the wizard sends when it keeps the key. Runs as root; safe to rerun,
# and a rerun with a key replaces it.
set -euo pipefail
key="${DEPLOY_PUBLIC_KEY?set DEPLOY_PUBLIC_KEY, or set it empty to keep the key}"
# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
keys="$root/home/deploy/.ssh/authorized_keys"
if [[ -n "$key" ]]; then
  [[ "$key" =~ ^ssh-ed25519\ [A-Za-z0-9+/=]+(\ [^\"]*)?$ ]] \
    || { echo "DEPLOY_PUBLIC_KEY is not one ssh-ed25519 public key" >&2; exit 1; }
elif [[ ! -s "$keys" ]]; then
  echo "no deploy key given, and no deploy key on the server to keep" >&2
  exit 1
fi

if ! id deploy >/dev/null 2>&1; then
  useradd --create-home --shell /bin/bash deploy
fi
passwd -l deploy >/dev/null

# Root owns the directory; `deploy` may only write the lock that keeps deploys to one
# at a time. Uploads never touch the disk as `deploy`: they go to filedrawer-install
# on stdin.
state="$root/var/lib/filedrawer-deploy"
mkdir -p "$state"
chown root:root "$state"
chmod 0755 "$state"
rm -rf "$state/incoming"
touch "$state/lock"
chown deploy:deploy "$state/lock"
chmod 0600 "$state/lock"
if [[ -n "$key" ]]; then
  mkdir -p "$(dirname "$keys")"
  chown deploy:deploy "$(dirname "$keys")"
  chmod 0700 "$(dirname "$keys")"
  # `restrict` turns off forwarding, the PTY, agent and X11; the forced command
  # ignores whatever the client asks to run and reads it from SSH_ORIGINAL_COMMAND
  # instead.
  printf 'restrict,command="/usr/local/bin/filedrawer-receive" %s\n' "$key" > "$keys"
  chown deploy:deploy "$keys"
  chmod 0600 "$keys"
fi

# A regular expression over the whole argument list (sudo 1.9.10 and later), not a
# wildcard, whose `*` would match spaces and so further arguments, such as
# `staging <sha256> production`. Checked before it goes in place.
rule="$root/etc/sudoers.d/filedrawer-deploy"
tmp=$(mktemp "$root/etc/sudoers.d/.filedrawer-deploy.XXXXXX")
trap 'rm -f "$tmp"' EXIT
cat > "$tmp" <<'CONF'
deploy ALL=(root) NOPASSWD: /usr/local/sbin/filedrawer-install ^staging [0-9a-f]{64}$
CONF
chmod 0440 "$tmp"
visudo -cf "$tmp" >/dev/null
mv "$tmp" "$rule"

echo "deploy user: done"
