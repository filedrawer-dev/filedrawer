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
- Chosen: hold them. Staging is the owner's alone, and step 4 builds embargoes in full.
- Against: an embargoed entry on staging stays hidden past its date until step 4 lands.

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
