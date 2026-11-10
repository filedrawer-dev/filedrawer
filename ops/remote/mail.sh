#!/usr/bin/env bash
# ENVIRONMENT's SMTP settings (staging or production), in their own file so
# service.sh never overwrites them. Each environment has a sending user of its own,
# so staging's password can be revoked alone. Reads the SMTP password as the first
# line of stdin; SMTP_USER and MAIL_FROM come as variables. Runs as root; safe to
# rerun, and a rerun replaces the settings. Exits 2 for an environment it doesn't
# know.
set -euo pipefail
case "${ENVIRONMENT-}" in
  staging | production) ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
: "${SMTP_USER:?set SMTP_USER}" "${MAIL_FROM:?set MAIL_FROM}"
SMTP_PASSWORD="${SMTP_PASSWORD:?pass the password as the first line of stdin}"
# systemd reads single-quoted values literally, so a value may hold anything but a
# single quote or a line break.
for value in "$SMTP_USER" "$MAIL_FROM" "$SMTP_PASSWORD"; do
  if [[ "$value" == *"'"* || "$value" == *$'\n'* ]]; then
    echo "a setting holds a single quote or a line break; for the password, make a new one without" >&2
    exit 1
  fi
done

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
file="$root/etc/filedrawer/$ENVIRONMENT-mail.env"
mkdir -p "$root/etc/filedrawer"
umask 077
cat > "$file" <<CONF
FILEDRAWER_SMTP_HOST=smtp.purelymail.com
FILEDRAWER_SMTP_PORT=465
FILEDRAWER_SMTP_USER='${SMTP_USER}'
FILEDRAWER_SMTP_PASSWORD='${SMTP_PASSWORD}'
FILEDRAWER_MAIL_FROM='${MAIL_FROM}'
CONF
unset SMTP_PASSWORD
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
systemctl try-restart "filedrawer-$ENVIRONMENT.service"

echo "mail ($ENVIRONMENT): done"
