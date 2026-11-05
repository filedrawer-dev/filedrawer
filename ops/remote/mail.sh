#!/usr/bin/env bash
# Staging's SMTP settings, in their own file so staging-service.sh never overwrites
# them. Reads the SMTP password as the first line of stdin; SMTP_USER and MAIL_FROM
# come as variables. Runs as root; safe to rerun, and a rerun replaces the settings.
set -euo pipefail
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

umask 027
cat > /etc/filedrawer/staging-mail.env <<CONF
FILEDRAWER_SMTP_HOST=smtp.purelymail.com
FILEDRAWER_SMTP_PORT=465
FILEDRAWER_SMTP_USER='${SMTP_USER}'
FILEDRAWER_SMTP_PASSWORD='${SMTP_PASSWORD}'
FILEDRAWER_MAIL_FROM='${MAIL_FROM}'
CONF
unset SMTP_PASSWORD
chown root:filedrawer /etc/filedrawer/staging-mail.env
chmod 0640 /etc/filedrawer/staging-mail.env
systemctl daemon-reload
systemctl try-restart filedrawer-staging.service

echo "mail: done"
