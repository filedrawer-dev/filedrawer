# Revoking every account's secrets after a leak

When secrets may have leaked, `filedrawer admin revoke-all` ends them for every account at once, the admin's included (security review, item 7; design.md, Admin).
Pick what to revoke by what leaked:

- **Live secrets**, such as session cookies, API tokens or email links in a log, a crash dump or a compromised process: run it without `--passwords`.
  Sessions, tokens and email links are stored only as hashes of random secrets, so a copy of the database alone doesn't call for this.
- **Password hashes**, such as a copy of the database or of a nightly backup: add `--passwords`.
  Weak passwords can be cracked offline, and a forced login alone would let a cracked one back in.
  Every account then has no password until its owner chooses one through `/reset`; a login meanwhile gets the answer a wrong password gets.
  An account that signed up with GitHub has no password to lose, needs no reset, and an account linked to GitHub can still log in with GitHub.
  The same copy holds the invite codes as they are: make new codes for any sprint still running, and revoke the old ones at `/admin/invites`, which freezes the unscreened entries of the accounts that joined with them.
- **The server's settings**, such as a copy of `/etc/filedrawer` or a compromised server: also rotate the GitHub OAuth App's client secret, as below.
  The database holds no GitHub token, since the server revokes each one as soon as it has read the account, so the client secret is the only GitHub sign-in secret to rotate.
  The other secrets there, such as the link checks' GitHub token, the Anthropic API keys and the SMTP passwords, are each replaced by their stage of `ops/provision-vps --ask-all`, after you revoke the old one where it was made.

Nobody is emailed.
Announce it yourself, and say that people must give their agents a new token, and with `--passwords` choose a new password through `/reset`, unless they log in with GitHub.
Every session ends either way: people who log in with GitHub just log in with GitHub again.

## Run it

From your own machine, with the server's address in place of `ADDRESS` (`VPS_HOST` in your wizard checkout's `ops/.vps.env`).
First run it as below, which changes nothing and prints what it would end; then run it again with ` --yes` added at the end, which does it in one transaction.
Drop `--passwords` for a leak of live secrets only.

Production:

```bash
ssh root@ADDRESS runuser -u filedrawer-prod -- \
  env FILEDRAWER_DATABASE=/srv/filedrawer/production/data/filedrawer.db \
  /srv/filedrawer/production/bin/filedrawer \
  admin revoke-all --passwords
```

Staging:

```bash
ssh root@ADDRESS runuser -u filedrawer -- \
  env FILEDRAWER_DATABASE=/srv/filedrawer/staging/data/filedrawer.db \
  /srv/filedrawer/staging/bin/filedrawer \
  admin revoke-all --passwords
```

It refuses a database with no accounts, which is what a mistyped path opens.
A server busy writing for more than 5 s makes it fail with "database is locked" and change nothing; run it again.
The running server sees the change at once: it reads sessions and tokens from the database on every request.

## After it

- Reset your own password through `/reset`, then log in; the admin pages need a session.
- Accounts made with `admin create-account` have no password and need `admin create-token` again.
- On staging, CI's smoke-test and screen-test accounts lose their tokens, and so does the token agents use as `FILEDRAWER_STAGING_API_KEY`.
  `ops/provision-vps --ask-all smoke` makes new ones for CI: answer yes to replacing `SMOKE_TEST_TOKEN` and `CI_SCREEN_TOKEN`.
- A reset or verification email queued before the run still goes out with a working link, since the link is made when the email is sent.

## Rotating the GitHub OAuth App's client secret

Each environment has an OAuth App of its own, under the `filedrawer-dev` organization's settings, OAuth apps, so rotate only the one whose server's settings leaked.

1. On the app's page, click "Generate a new client secret" and copy it.
2. From your wizard checkout, give it to the server, which restarts the service: `ops/provision-vps --ask-all production github-oauth` for production, `ops/provision-vps --ask-all github-oauth` for staging.
   Answer yes to changing the client secret, and Enter to keep the client ID.
   The run ends by checking the new secret with GitHub.
3. Back on the app's page, delete the old secret.
   Until then it still works for anyone who holds it.

Sessions are untouched, since the client secret plays no part in them.
