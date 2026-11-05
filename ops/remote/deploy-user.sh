#!/usr/bin/env bash
# The `deploy` user GitHub Actions logs in as (design.md, Hosting and operations).
# Its one key may only run filedrawer-receive, which hands an upload to
# filedrawer-install through the one sudo rule below. DEPLOY_PUBLIC_KEY is the key's
# public half; the wizard has already copied both scripts into place. Runs as root;
# safe to rerun, and a rerun replaces the key.
set -euo pipefail
: "${DEPLOY_PUBLIC_KEY:?set DEPLOY_PUBLIC_KEY}"
[[ "$DEPLOY_PUBLIC_KEY" =~ ^ssh-ed25519\ [A-Za-z0-9+/=]+(\ [^\"]*)?$ ]] \
  || { echo "DEPLOY_PUBLIC_KEY is not one ssh-ed25519 public key" >&2; exit 1; }

if ! id deploy >/dev/null 2>&1; then
  useradd --create-home --shell /bin/bash deploy
fi
passwd -l deploy >/dev/null

# Root owns the directory; `deploy` may only write the lock that keeps deploys to one
# at a time. Uploads never touch the disk as `deploy`: they go to filedrawer-install
# on stdin.
install -d -o root -g root -m 0755 /var/lib/filedrawer-deploy
rm -rf /var/lib/filedrawer-deploy/incoming
touch /var/lib/filedrawer-deploy/lock
chown deploy:deploy /var/lib/filedrawer-deploy/lock
chmod 0600 /var/lib/filedrawer-deploy/lock
install -d -o deploy -g deploy -m 0700 /home/deploy/.ssh
# `restrict` turns off forwarding, the PTY, agent and X11; the forced command ignores
# whatever the client asks to run and reads it from SSH_ORIGINAL_COMMAND instead.
printf 'restrict,command="/usr/local/bin/filedrawer-receive" %s\n' "$DEPLOY_PUBLIC_KEY" \
  > /home/deploy/.ssh/authorized_keys
chown deploy:deploy /home/deploy/.ssh/authorized_keys
chmod 0600 /home/deploy/.ssh/authorized_keys

cat > /etc/sudoers.d/filedrawer-deploy <<'CONF'
deploy ALL=(root) NOPASSWD: /usr/local/sbin/filedrawer-install staging *
CONF
chmod 0440 /etc/sudoers.d/filedrawer-deploy
visudo -cf /etc/sudoers.d/filedrawer-deploy >/dev/null

echo "deploy user: done"
