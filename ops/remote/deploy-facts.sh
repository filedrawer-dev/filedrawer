#!/usr/bin/env bash
# What ops/deploy-production (#42) needs to know of the server before it changes
# anything, read in one ssh round as root, one `name value` line each:
#   caddy    what `caddy version` prints
#   schema   production's schema version (the highest in _sqlx_migrations), `none`
#            when there's no database file, `error` when it can't be read
#   binary   the "filedrawer-commit <sha>" production's binary embeds, or `none`
#   install  the SHA-256 of /usr/local/sbin/filedrawer-install, or `none`
#   backup   the SHA-256 of /usr/local/sbin/filedrawer-backup, or `none`
# The database is read as production's user, read-only, as the install reads it.
set -uo pipefail

database=/srv/filedrawer/production/data/filedrawer.db
binary=/srv/filedrawer/production/bin/filedrawer

# The version line, past any warning Caddy prints first; else whatever it said.
caddy_said=$(caddy version 2>&1)
echo "caddy $(grep -m 1 -E '^v[0-9]' <<< "$caddy_said" || head -n 1 <<< "$caddy_said")"
if runuser -u filedrawer-prod -- test -e "$database"; then
  schema=$(runuser -u filedrawer-prod -- sqlite3 -bail "file:$database?mode=ro" \
    ".timeout 10000" "SELECT max(version) FROM _sqlx_migrations;" 2>/dev/null) || schema=error
  [[ "$schema" =~ ^[0-9]+$ ]] || schema=error
else
  schema=none
fi
echo "schema $schema"
commit=""
[[ -f "$binary" ]] && commit=$(grep -aoE 'filedrawer-commit [0-9a-f]{40}' "$binary" | head -n 1)
echo "binary ${commit:-none}"
for name in install backup; do
  file=/usr/local/sbin/filedrawer-$name
  if [[ -f "$file" ]]; then
    echo "$name $(sha256sum "$file" | cut -d' ' -f1)"
  else
    echo "$name none"
  fi
done
