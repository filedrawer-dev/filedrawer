#!/usr/bin/env bash
# ENVIRONMENT's SMTP settings (staging or production), in their own file so
# service.sh never overwrites them. Each environment has a sending user of its own,
# so staging's password can be revoked alone. Reads the SMTP password as the first
# line of stdin; SMTP_USER and MAIL_FROM come as variables. An empty password keeps
# the one stored for the same SMTP_USER, and an empty SMTP_USER or MAIL_FROM keeps
# the stored one (#112). Runs as root; safe to rerun, and the service restarts only
# when the settings changed. Exits 2 for an environment it doesn't know.
set -euo pipefail
case "${ENVIRONMENT-}" in
  staging | production) ;;
  *) echo "ENVIRONMENT is '${ENVIRONMENT-}'; it takes staging or production" >&2; exit 2 ;;
esac
SMTP_USER="${SMTP_USER-}" MAIL_FROM="${MAIL_FROM-}" SMTP_PASSWORD="${SMTP_PASSWORD-}"

# The override is for tests, which point it at a fake root.
root="${FILEDRAWER_ROOT:-}"
file="$root/etc/filedrawer/$ENVIRONMENT-mail.env"
stage=mail
[[ $ENVIRONMENT == staging ]] || stage="production mail"

# stored NAME: NAME's single-quoted value in the file, if any.
stored() { sed -n "s/^$1='\(.*\)'\$/\1/p" "$file" 2>/dev/null || true; }
[[ -n "$SMTP_USER" ]] || SMTP_USER=$(stored FILEDRAWER_SMTP_USER)
[[ -n "$MAIL_FROM" ]] || MAIL_FROM=$(stored FILEDRAWER_MAIL_FROM)
if [[ -z "$SMTP_USER" || -z "$MAIL_FROM" ]]; then
  echo "no sending user or From line given, and none stored; run ops/provision-vps --ask-all $stage" >&2
  exit 1
fi
# A password belongs to its user, so another user gets none of it.
if [[ -z "$SMTP_PASSWORD" && "$(stored FILEDRAWER_SMTP_USER)" == "$SMTP_USER" ]]; then
  SMTP_PASSWORD=$(stored FILEDRAWER_SMTP_PASSWORD)
fi
if [[ -z "$SMTP_PASSWORD" ]]; then
  echo "no password given, and none stored for $SMTP_USER; to give one, run ops/provision-vps --ask-all $stage" >&2
  exit 1
fi
# systemd reads single-quoted values literally, so a value may hold anything but a
# single quote or a line break.
for value in "$SMTP_USER" "$MAIL_FROM" "$SMTP_PASSWORD"; do
  if [[ "$value" == *"'"* || "$value" == *$'\n'* ]]; then
    echo "a setting holds a single quote or a line break; for the password, make a new one without" >&2
    exit 1
  fi
done

mkdir -p "$root/etc/filedrawer"
umask 077
cat > "$file.new" <<CONF
FILEDRAWER_SMTP_HOST=smtp.purelymail.com
FILEDRAWER_SMTP_PORT=465
FILEDRAWER_SMTP_USER='${SMTP_USER}'
FILEDRAWER_SMTP_PASSWORD='${SMTP_PASSWORD}'
FILEDRAWER_MAIL_FROM='${MAIL_FROM}'
CONF
unset SMTP_PASSWORD
# A rerun that keeps everything leaves the service running.
if cmp -s "$file.new" "$file"; then
  rm -f "$file.new"
  echo "mail ($ENVIRONMENT): unchanged"
  exit 0
fi
mv -f "$file.new" "$file"
chown root:root "$file"
chmod 0600 "$file"
systemctl daemon-reload
systemctl try-restart "filedrawer-$ENVIRONMENT.service"

echo "mail ($ENVIRONMENT): done"
