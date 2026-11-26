# Restoring production from a Hetzner backup

When production's data is lost or damaged, or its server is gone, the beta's way back is a Hetzner backup and the nightly copies of the database inside it (#41; design.md, Hosting and operations).
`ops/restore-drill.md` practises the restore on a throwaway server; this is the real thing.
Everything written after the copy you restore is lost, up to a day of it.

Two servers must never run as production at once: both would screen with production's key, send mail and ping healthchecks.io.
So power the old server off before the restored one boots, unless it is gone already.

## 1. Pick the way back

- **A.
  The server is up, but its disk or data is bad.**
  Rebuild it from a backup.
  It stays the same server, with the same IP addresses, so DNS, `VPS_HOST` and CI's settings stay as they are.
- **B.
  The server is gone, or you want to keep it as it is to see what happened.**
  Create a new server from a backup, then either move the old server's IP addresses to it or point DNS and CI at its new ones.
  Moving the addresses changes nothing else, so prefer it while the old server still exists.

Either way, open production's server in the Hetzner Cloud console, then Backups, and pick the newest backup taken before the damage.
Note its date and time.

## 2A. Rebuild the server from the backup

On the backup, choose Rebuild.
It overwrites the server's disk.
The server boots with production's units running on the backup's database, so go straight on to step 3.

## 2B. A new server from the backup

1. Power the old server off, if it still runs.
2. On the backup, choose "Create server from backup".
   Pick production's location, a type with at least production's disk, and production's firewall, if it has one in the console.
   The server boots as production, on new IP addresses.
3. To keep the old addresses:
   1. Power the new server off.
   2. Under Primary IPs, unassign the old server's IPv4 and IPv6 and the new server's.
      Hetzner moves a Primary IP only between powered-off servers in the same location, and a server holds at most one of each type ([Hetzner's Primary IP FAQ](https://docs.hetzner.com/cloud/servers/primary-ips/faq/)).
   3. Assign the old server's two to the new one, and power it on.
   4. Delete the new server's own two addresses, which Hetzner bills while they exist, assigned or not.
4. Or, to use the new addresses:
   1. At Cloudflare, change the A and AAAA records of `filedrawer.dev` and `staging.filedrawer.dev` to the new server's addresses, still "DNS only".
   2. In your wizard checkout's `ops/.vps.env`, set `VPS_HOST` to the new address, then rerun `ops/provision-vps`.
      It rewrites the GitHub variables `VPS_HOST` and `VPS_KNOWN_HOSTS`, the latter from the host key it reads over your root login, so a deploy reaches the new server.

`ssh root@<address>` may warn that the host key changed, if the new server made new ones at its first boot.
Check the key in the console's server view before you accept it, then remove the old line from your `known_hosts`.

## 3. Check the database, or put the nightly copy in its place

The backup holds production's live database as Hetzner found it, up to the backup's time, and the two nightly copies in `backups/`, each made whole by `VACUUM INTO` at 23:30 UTC.
The live file is newer, but Hetzner may have caught it mid-write.
Stop production, then check it as production's user, never as root: root's `sqlite3` would leave `-wal` and `-shm` files the server can't open.

```sh
systemctl stop filedrawer-production
runuser -u filedrawer-prod -- sqlite3 /srv/filedrawer/production/data/filedrawer.db 'PRAGMA integrity_check'
```

If it prints `ok`, keep it.
Otherwise put the newest nightly copy dated before the backup in its place:

```sh
cd /srv/filedrawer/production/data
ls -l backups/
runuser -u filedrawer-prod -- rm -f filedrawer.db-wal filedrawer.db-shm
runuser -u filedrawer-prod -- cp backups/filedrawer-YYYY-MM-DD.db filedrawer.db
```

Then `systemctl start filedrawer-production`.

## 4. Check the site before and after it takes traffic

From your own machine, before DNS or the addresses point at the server, ask its Caddy by name:

```sh
curl -s -o /dev/null -w '%{http_code}\n' --resolve filedrawer.dev:443:<address> https://filedrawer.dev/tags
```

A bare `https://<address>` gets nothing, since Caddy answers only for the sites' names.
Once it takes traffic:

- `https://filedrawer.dev/tags` answers 200, and UptimeRobot says it's up.
- `ops/audit-vps` reports the server as you expect, with each root-owned script matching `origin/main`.
- `ops/smoke-staging read https://filedrawer.dev` passes, the read-only checks `ops/deploy-production` runs.
- The next night's copy appears in `backups/`, and its healthchecks.io check goes green.

## 5. Redo what the restore undid

The restore reverts every write since the database you kept:

- Entries, signups, sessions and password changes since then are gone.
  Links in emails sent since then point at tokens the restored database lacks, so they fail.
- Removals, suspensions, file bans and purges since then are undone.
  Redo each one, and every purge above all, since the privacy page promises erasure.
  The server applies `purges.log`, beside the database, again when it starts, which redoes the purges only when you restored the database file alone and kept the disk's log.
  A restore of the whole server, such as from Hetzner's backups, rolls the log back too, so redo every purge since then by hand: the log leaves the disk only with #4.
  A GitHub ID you typed on a purge's page isn't in the log, so purge it again after either kind of restore.
- Mail waiting in the queue at the copy's time goes out again, so a person may get a second email with a link that fails.
- Attached files uploaded since then stay on disk with nothing naming them.

Write on #41, or a new issue, what failed, which backup and database you used, and what you redid.

Losing the Hetzner account loses the backups too; the beta accepts that until #4 lands.
