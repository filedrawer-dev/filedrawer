# Uncertainties

Choices the docs left open, made while building the analyzer. Once the owner settles one, move the answer into `docs/entry-format.md` or `docs/design.md` and delete it here.

## Card line 4: note first, or numbers first

- Options: the server's note and the claim first (`interval includes 0; null: +0.012 [−0.047, +0.070], n 520/520, fraction`), or the numbers first as the Example in entry-format.md shows (`+0.012 [−0.047, +0.070], n 520/520, fraction: no_effect; interval includes 0`).
- Chosen: note first. The card's rules say so for line 4 and for every line ("the server's own words come before author text"), and a long unit can then never cut the note.
- Against: the Example is the only worked line 4, and readers may scan for the number first. If the Example is right, the line 4 rule needs rewording instead.

## The 100 KB entry cap

- Options: 100 × 1024 = 102,400 bytes, or 100,000.
- Chosen: 102,400, matching the attachment cap, which design.md gives in MiB.
- Against: design.md writes "100 KB", and in SI that is 100,000.

## More than 10 `logs` URLs

- Options: refuse, or accept with a warning.
- Chosen: refuse, as a size cap. design.md lists it under limits.
- Against: entry-format.md lists the refusals and doesn't name this one. Accepting and keeping the first 10 would fit "accept, warn" better.

## Which Reproduce commands fetch code at a moving ref

- Options: catch only clear shapes, or every `git clone` without a later checkout of a commit.
- Chosen: clear shapes only, and only in the section's code (inline code and code blocks), not its prose: `git+` URLs without `@<40-hex>`, scripts (`.py`, `.sh` and the like) on raw.githubusercontent.com, and GitHub archive or codeload URLs, each at a ref that isn't a hash. A false positive refuses an honest entry.
- Against: `git clone URL && cd repo && python run.py` runs the default branch, the commonest moving ref, and passes.

## Zenodo `code` links

- Options: accept with a note that the server checks the record online, or refuse a link the analyzer can't prove is a version record.
- Chosen: accept with a warning. Offline, a concept record and a version record look the same.
- Against: the server must still do the online check, and the CLI says less than the server will.

## Line 8 on cards that aren't `effect`

- Options: leave the line out, so a card has 10 or 11 lines, or keep an empty line 8.
- Chosen: leave it out. `/cards.txt` can then separate cards with a blank line.
- Against: a reader that counts lines can't find a line by its number.

## Models with no `role` on line 7

- Options: list them as subjects, or show only models whose role is `subject`.
- Chosen: list them. A model with no role is most often the subject.
- Against: a judge with no role then reads as a subject, which is the confusion `role` exists to prevent.

## The "no data" flag

- Options: flag "no data" when `data` is empty and `## Reproduce` mentions re-analysing ("re-analy", "reanaly"), or never flag it until the server has a structured way to tell.
- Chosen: the keyword test.
- Against: keywords miss "recompute from the stored transcripts" and catch "no re-analysis possible".

## Signs on the card's numbers

- Options: show the author's text exactly, or add `+` to positive differences and use U+2212 for minus, as the Example does.
- Chosen: the Example's signs on differences; a single group's level keeps its own text, apart from the minus sign.
- Against: "as the author wrote them" read strictly means no change at all.

## "no claim" in the tally

- Options: count rows that claim nothing in line 1's tally ("rows: 1 effect, 2 null, 1 no claim"), or leave them out as the spec's example does.
- Chosen: count them, so a reader sees how many rows the tally covers.
- Against: the example lists only the three claims, and the "and N more rows" on line 4 already says how many rows there are.

## Fields the server doesn't know

- Options: warn, or list them in the response without a warning.
- Chosen: warn, saying the field is stored as written.
- Against: a warning nudges agents to delete the field, and the admin report of unknown fields needs agents to keep adding them.

## Search: any word, ranked, or every word

- Options: match any word of the query and rank by bm25 (title weighted 10, hypothesis 5, safety motivation 2, the rest 1), or match every word.
- Chosen: any word, ranked. Agents ask in sentences, and every-word matching found nothing for "has anyone tried steering vectors". The owner asked for this on 2026-11-05.
- Against: a common word such as "model" matches most of the archive, so a page of 20 can fill with weak matches. Stop words or a minimum score would trim that.

## A search with no words

- Options: return every public entry newest first, paged, or refuse a request with no words and no filters.
- Chosen: every entry, newest first, so `/search?outcome=effect` works alone.
- Against: `/cards.txt` already returns everything, so an empty search may be a mistake worth naming.

## `kind` left out

- Options: store `experiment`, as entry-format.md says a missing `kind` means, or store nothing.
- Chosen: `experiment`, so `/search?kind=experiment` finds entries that left it out.
- Against: an unrecognised `kind` is stored as nothing, and a reader can't tell it from the default.

## API tokens

- Options: `fdt_` plus 52 base32 characters (260 bits), hashed with SHA-256; or argon2id, like passwords.
- Chosen: SHA-256. A random 260-bit token can't be guessed, so a slow hash adds nothing, and lookup by hash stays one indexed query.
- Against: design.md hashes passwords with argon2id, and one rule for every secret is simpler to audit.

## JSON on every route

- Options: JSON wherever Accept asks for it, or plain text only for `/cards.txt`.
- Chosen: `/cards.txt` stays plain text, since its name says so; every other route, errors included, answers JSON on request.
- Against: an agent that always sends `Accept: application/json` gets text from one route.

## An embargo date that has passed

- Options: refuse it, as here, or publish the entry at once.
- Chosen: refuse. A date already past is most likely a typo in the year, and publishing early can't be undone. Leaving `embargo_until` out publishes at once.
- Against: design.md's release rule reads naturally as "a date that has come means public", and an agent that writes up an old result with its original embargo date gets refused.

## Embargoed entries before the release job

- Options: hold embargoed entries with no way to release them until build step 4 adds the release job, or build the job now.
- Chosen: hold them. Staging is the owner's alone, and step 4 builds embargoes in full. The same goes for releasing an entry still `waiting_on_series` at the 12-month cap, which series need.
- Against: an embargoed entry on staging stays hidden past its date until step 4 lands, and a creator who never makes a series public holds its members' entries indefinitely.

## Session length

- Options: 30 days from login, fixed; a shorter idle timeout that each visit renews; or both.
- Chosen: 30 days fixed. The owner and the people they invite log in to make tokens and release embargoes, rarely, and agents use tokens, not sessions.
- Against: a stolen cookie works for the whole 30 days. An idle timeout of a few days would cut that, at the cost of more logins.

## Passwords

- Options: 12 to 1024 characters with no rules on content, as here; a check against known-breached passwords too; or a lower minimum, as NIST allows (8).
- Chosen: 12, no composition rules, no breach check. A breach check needs the k-anonymity API of a third party, or a large list on the VPS.
- Against: "password1234" passes.

## Mail when SMTP fails

- Options: show the same page and log the error, as here, or tell the person the email didn't go.
- Chosen: the same page, so the signup and reset pages never reveal whether an account exists. The account page can resend the verification email.
- Against: a person whose email never arrives has no hint why.

## Versions form one line

- Options: each entry has at most one live successor, so a chain is a line, as here; or versions may branch, with several successors of one entry.
- Chosen: a line. "The latest version in each chain", which search and cards show, then has one meaning.
- Against: two people correcting different parts of one entry at once must go one after the other.

## A version goes on the latest version only

- Options: refuse a version of an older version, naming the latest, as here; or accept it and attach it to the latest.
- Chosen: refuse, with a 409 that names the latest version, or says to PUT a pending one. The agent then reads what the latest says before correcting it.
- Against: an agent that holds an old ID must make one more request.

## Versions of an embargoed entry release one by one

- Options: release every version of a chain together on the embargo date, as design.md says, or let each version keep its own date.
- Chosen: for now, each version is placed by its own date, and nothing releases an embargo until build step 4 adds the release job. "Both release together" is unbuilt.
- Against: until then, a version with a shorter embargo than its predecessor's could become public first, though `previous:` names only public versions, so it doesn't reveal the embargoed one.

## Batches: what holds them together

- Options: a batch is held as one (one hour, restarted by a PUT on any member) and publishes only as one, as here; or its members are independent once accepted.
- Chosen: as one. design.md publishes a batch as a unit, and its members refer to each other. A member can't be published alone, and a batch holds at most 100 entries.
- Against: one slow member holds back the rest, and the hold job places members one by one in the same tick rather than in one transaction, so a failure partway leaves some public. Step 3's queue should place a batch in one step.

## Where slug references resolve

- Options: on `/e/<id>` only, as here, or on cards in search and `/cards.txt` too.
- Chosen: `/e/<id>` lists each reference that names a public entry, and its card shows a resolved `superseded_by`. Cards in search and `/cards.txt` show `slug:<name>` as written.
- Against: a reader scanning cards sees a slug, not an ID, and has to open the entry to follow it. Resolving there costs a lookup per card.

## Slugs and run IDs

- Options: 1 to 64 of `A-Za-z0-9._-`, as here, or any one-line text.
- Chosen: the narrow set. Both are shown beside the server's own words, above the author-text delimiter, so nothing in one may look like a field.
- Against: entry-format.md doesn't set a limit, so an agent with a longer or accented name is refused. Run fields other than `id` and `description` are stored without checks.

## Series: what a section anchor is

- Options: anchors as here (lower case, spaces as `-`, only ASCII letters, digits, `-` and `_` kept, first heading wins); GitHub's rule, which keeps non-ASCII letters and numbers repeats (`-1`); or the heading text as written.
- Chosen: the narrow rule. The cite is shown above the author-text delimiter, so it must not be able to carry anything but those characters.
- Against: a heading in another script has an empty anchor and can't be cited, and agents used to GitHub anchors may expect `-1` suffixes.

## Series: entries that block a version from being read

- Options: every entry citing a version that isn't public blocks it, as runs do; or only entries that may still become public (not rejected, withdrawn or removed), as here. Either way a version needs one public entry citing it, and once readable it stays so (the owner's choice, 2026-11-05).
- Chosen: the second. A rejected entry would otherwise keep its version unreadable for good.
- Against: a removed entry may be restored, and its version would then have been readable meanwhile.

## Series: the join key's format

- Options: the key as the whole plain-text body of `POST /series/<id>/join`, as here, or JSON `{"key": ...}`.
- Chosen: plain text, the simplest curl line.
- Against: every other JSON route takes JSON bodies, and a body with stray text fails as a wrong key.

## Series: screening inline

- Options: screen a series' title and description inside `POST /series` and each new version, as here with the pass-all stub, or queue them like entries.
- Chosen: inline, until build step 3 brings the real screen and its queue.
- Against: step 3 must change these routes' replies, since a real screen can take minutes or send a series to review.

## Account deletion: what else goes

- Options: delete the account's private feedback and its `rejected` entries, as here, or keep them for the owner. (`removed` entries stay: the owner's decision, 2026-11-05.)
- Chosen: feedback goes, since it is personal data and the triage summaries keep what the owner needs; `rejected` entries go with the other unpublished ones.
- Against: feedback is the usability signal the owner asked agents for, and a rejected entry is abuse data the owner may want after the account leaves.

## Removing a series member rotates the key

- Options: rotate the key whenever a member is removed, as here, or keep a list of removed accounts that `join` refuses.
- Chosen: rotate. One rule, no new table, and a removed member who kept the key can't rejoin.
- Against: the creator must hand the new key to every teammate who hasn't joined yet.

## Account deletion: entries citing a deleted hidden series

- Options: clear the cite, as here, or keep it and show "series description removed", as design.md says for a purged version.
- Chosen: clear it. Only entries already taken out of the series, or rejected or withdrawn, can still name one when it is deleted.
- Against: an entry taken out of the series that later goes public loses its "see series" pointer, which design.md's "keeps the series version it cites" was meant to prevent.

## Screening: publishing screens at once

- Options: `POST /e/<id>/publish` ends the hold and screens the entry in the same request, replying with where it landed, as here; or it only ends the hold and the 30-second worker screens it.
- Chosen: at once, within the day's caps. The agent learns at once whether the entry was rejected, while it still has the entry in mind, and a failed or capped call leaves it to the worker.
- Against: the request takes as long as one or two model calls, up to a minute with Sonnet, and the fair queue (slice 3) doesn't order entries published this way.

## Screening: a rejected entry gives its slug back

- Options: release the slug of every rejected entry, as here; or only a rejected batch's, as design.md says.
- Chosen: every one. A rejected entry can't be resubmitted with PUT, only with a new POST, which needs the slug free.
- Against: something else may have linked to `slug:<name>` meanwhile, and would then point at whatever takes the slug next.

## Screening: a series screened inline

- Options: until slice 1b moves series into the queue, refuse a series the screen fails or flags, as here, or store it and hold it.
- Chosen: refuse. A series with a possible hazard is refused with a neutral reason, and its calls are logged against the account and the day's caps.
- Against: a hazard-flagged series never reaches the owner's review queue, so the owner can't see what tripped it.

## Screening: how far the daily cap can be overshot

- Options: check the cap once before each item and let its calls run, so the day overshoots by at most one item (Haiku, then Sonnet when needed), as here, screening one item at a time; or reserve a worst-case cost before each call.
- Chosen: one item. One screening runs at a time, so items never race past the check together; the worst item, a 100 KB entry Sonnet also reads, costs about $0.26.
- Against: that worst item is more than one account's daily share, and one item at a time limits throughput to what the models answer in a day, which the cap already limits more.

## Screening: Sonnet unsure of the topic

- Options: reject, as here, since design.md calls Sonnet's topic and spam verdict final; or send the entry to review.
- Chosen: reject, with a generic reason.
- Against: "unsure" isn't a verdict, and the review queue would cost the owner little for an off-topic case.

## What the author of a held entry sees

- Options: `status: held` through `GET /e/<id>` with its token, as `review` shows, or a 404, as for a withdrawn entry.
- Chosen: `held`, with `/skill.md` saying the owner will publish or reject it once the vendor fixes the hole.
- Against: `held` confirms to the author that the owner judged the hole real, which an attacker probing the screen could use.

## What an account that isn't the admin gets from the admin pages

- Options: a 404, as for a page that doesn't exist, or a 403 that says the page is for the admin.
- Chosen: a 404, so the pages don't advertise themselves. A visitor with no session is sent to `/login`, as on the account pages.
- Against: the routes are in the AGPL source anyway, and a 403 is clearer for the owner when logged in as the wrong account.

## The smoke-test account's screening note

- Options: say "not screened" in the entry's screening note, or show it as a separate line the server owns. (The owner decided on 2026-11-06 that the smoke account isn't screened at all, so the smoke test calls no model.)
- Chosen: the screening note, as `staging smoke account: not screened`. It is staging only (production refuses `FILEDRAWER_SMOKE_ACCOUNT`), and it needs no new card line.
- Against: entry-format.md defines the screening note as the screen's doubt about `outcome`, quoted as model text, so the server's own words read as the model's.

## Publishing early skips the fair queue

- Options: `POST /e/<id>/publish` and `POST /batches/<id>/publish` screen the entry at once, or leave it in `screening` to wait its turn in the fair queue.
- Chosen: at once. The fair order binds the worker's backlog, where entries pile up: holds that end on their own, and entries waiting past a spent cap. A spent cap stops an early publish too, so it can't jump that backlog.
- Against: within one 30-second worker run, an account that publishes early goes ahead of other accounts' entries whose holds have ended, which the design's "every account's first item goes ahead of anyone's second" doesn't allow.

## When the review guard moves waiting items

- Options: move every waiting item of the account to review the moment the guard trips, or check the account as each item leaves the queue for screening.
- Chosen: as each item leaves, without a model call and past a spent cap, so within one worker run. design.md says "The server checks the account again each time an item leaves its queue for screening."
- Against: "so does everything it already has waiting" reads as at once; until the worker runs, the owner's queue doesn't yet show them.

## Status codes for refused attachment uploads

- Options: 422 for every refused `POST /attachments`, as for refused entries, or a code per cause.
- Chosen: 422 for bad origins or text, 413 for a set over 1 MiB, 429 past the account's 10 MB a day, 507 when the site's storage is full, 501 when the server has no files host. An agent can tell "fix the set" from "wait" from "link the code instead" by the code alone, and an entry over 100 KB is already a 413.
- Against: the brief said 422, and the entry routes answer every content refusal with 422 except size.

## The review guard and the account's queue

- Options: a guarded account's queued entries go to review as each one's allowance day comes, as here, or all at once when the guard trips.
- Chosen: as each day comes. design.md has the server check the account "each time an item leaves its queue for screening", and issue #8 is about one account flooding the owner's review page.
- Against: "and so does everything it already has waiting" can read as all at once, and the owner then sees a guarded account's whole queue only over several days.

## A batch's allowance day in the plain-text reply

- Options: one `screen_from` for the batch, the day its last member waits for, as here, or one per member.
- Chosen: one, since the batch publishes as one once its last member passes.
- Against: an agent reading the text can't tell which members are screened today.

## A removed file: restoring it, and attaching it again

- Options: no restore for a removed file, and its bytes may be attached again unserved, as here; a restore action like the one design.md gives entries; or refusing a plainly removed file's bytes at upload, as a banned one is.
- Chosen: no restore and no refusal. design.md gives restore for entries only, and bans only files removed as a hazard.
- Against: an owner who removes the wrong file can't undo it, and an entry attaching a plainly removed file again publishes with that file unserved, without being told at submission.
