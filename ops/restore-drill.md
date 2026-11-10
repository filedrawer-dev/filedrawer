# Restore drill: a Hetzner backup and the newest nightly copy

The beta's backups are Hetzner's daily server backups and production's two nightly copies of its database in `backups/` (#41; design.md, Hosting and operations).
This drill proves they restore, once by hand, before the beta relies on them.
It takes about half an hour and costs a few cents of server time.

A server restored from a backup boots with production's units enabled.
Without a way out it would mail the owner, ping healthchecks.io's checks as production, and call Anthropic and GitHub with production's keys.
So the drill server gets a firewall that lets nothing out, attached when the server is created.

## 1. A firewall that lets nothing out

In the Hetzner Cloud console, under Firewalls, create `filedrawer-drill`:

- Inbound: TCP 22 from your own IP address only.
- Outbound: one rule, ICMP to `192.0.2.1/32` (a documentation address that nothing answers).
  A firewall with no outbound rule allows all outbound traffic; with one, it denies everything the rules don't name.

## 2. A server from the newest backup

1. Open production's server, then Backups, and note the newest backup's date and time.
2. On that backup, choose "Create server from backup".
   Pick the same location, any type with at least production's disk, and under Firewalls pick `filedrawer-drill`.
   Name it `filedrawer-drill`.
3. Once it's running, `ssh root@<its IP>`.
   Production's root key is on the restored disk, so your key logs in.

## 3. Stop production's units on the drill server

```sh
systemctl stop filedrawer-backup.timer filedrawer-production.service filedrawer-staging.service caddy
systemctl disable filedrawer-backup.timer filedrawer-production.service filedrawer-staging.service caddy
```

At boot the timer ran the copy the backup missed, with nowhere to ping; that copy is dated today and was taken from the live file as Hetzner captured it.
The drill opens the newest copy not dated today instead, the last one the nightly timer made before the backup:

```sh
ls -l /srv/filedrawer/production/data/backups/
```

Note which copy you open, such as `filedrawer-2026-11-20.db`.

## 4. Open the copy with the production binary

Work on a scratch copy, so the server's migrations change nothing restored:

```sh
mkdir /root/drill
cp /srv/filedrawer/production/data/backups/filedrawer-YYYY-MM-DD.db /root/drill/filedrawer.db
ln -s /srv/filedrawer/production/data/files /root/drill/files
FILEDRAWER_DATABASE=/root/drill/filedrawer.db \
FILEDRAWER_LISTEN=127.0.0.1:9090 \
FILEDRAWER_ENVIRONMENT=staging \
FILEDRAWER_SCREENER=pass-all \
FILEDRAWER_LINK_CHECKS=off \
FILEDRAWER_PUBLIC_URL=http://127.0.0.1:9090 \
  /srv/filedrawer/production/bin/filedrawer serve &
curl -s -o /dev/null -w '%{http_code}\n' --retry 10 --retry-connrefused --retry-delay 1 \
  http://127.0.0.1:9090/tags
```

It runs as staging, with no ping and no SMTP settings, so it neither mails nor pings (the owner's decision, 2026-11-09).
`/tags` answers 200 once the server has run its migrations on the copy.

## 5. Check the entries and one attached file

```sh
sqlite3 /root/drill/filedrawer.db "SELECT status, count(*) FROM entries GROUP BY status"
```

The counts match production's on the copy's date, as the daily email after it reported, give or take that day's entries.

```sh
sha=$(sqlite3 /root/drill/filedrawer.db "SELECT sha256 FROM files LIMIT 1")
sha256sum "/root/drill/files/${sha:0:2}/$sha"
```

The hash it prints is the file's name.
With no attached files yet, `sha` is empty: note that, and check one at the next drill.

## 6. Delete the drill server

```sh
kill %1
```

Then delete `filedrawer-drill` in the console; it holds production's data.
Keep the firewall for the next drill.

Write on #41 the date, the backup and the copy you used, the counts, and anything that went differently from these steps.
