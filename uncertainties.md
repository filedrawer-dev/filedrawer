# Uncertainties

Choices the docs left open, made while building.
Once the owner settles one, move the answer into `docs/entry-format.md` or `docs/design.md` and delete it here.

Each entry sits under its group (Product: what an agent or person sees or must do; Architecture: how the site is built and run) and its topic.
Add a new entry at the end of the topic it belongs to, or start a topic when none fits, rather than at the end of the file, so two sessions adding entries seldom touch the same lines.

## Product

### Cards

#### Card line 4: note first, or numbers first

- Options: the server's note and the claim first (`interval includes 0; null: +0.012 [−0.047, +0.070], n 520/520, fraction`), or the numbers first as the Example in entry-format.md shows (`+0.012 [−0.047, +0.070], n 520/520, fraction: no_effect; interval includes 0`).
- Chosen: note first.
  The card's rules say so for line 4 and for every line that isn't author text alone ("the server's own words come before author text"), and a long unit can then never cut the note.
  The Example's line 4 was rewritten to match when the labels came in (analyzer v2).
- Against: readers may scan for the number first.

#### The card's labels (analyzer v2)

- Options, for the words: the eleven chosen, `entry`, `title`, `hypothesis`, `result`, `design`, `stats`, `models`, `first caveat`, `not tried`, `flags`, `author`; or longer ones that name each line's contents (`primary row`, `control and prediction`, `baseline and method`).
  For where they go: before every line, lines 7 to 9 keeping the label they had; or only before the lines that started with author text (2, 3 and 11).
  For the caps: a cap counts what follows the label; or the whole line.
- Chosen: the eleven short words before every line, so the rule a reader holds is one sentence with no exceptions (the owner's decision 2026-11-21 was "every line"; the words are mine).
  Line 4 is `result`, not `primary row`, since the submission and batch replies print their own `primary row:` line just above the card.
  A cap counts what follows the label, so author text keeps the room it had under v1; lines 7 to 9, whose label counted under v1, gain its length.
  The deleted account's line reads `author: account <id>, account deleted`.
- Against: `design` and `stats` are vague for what they hold (positive control, prediction and S; baseline, level, method and unit).
  Line 1's `entry:` has the shape of the batch reply's `entry: <id> <url>` lines, though those are above the delimiter and the card's are indented below it.
  Linked handles, once linking lands, would follow the display name on line 11, so a name holding `, github:x` could pass for a handle; handles before the name would close that.

#### Stored entries' cards under v2

- Options: render every card at read time with the current analyzer, as before, so old entries show v2 at once; or keep rendering an entry with the analyzer version it was submitted under, which would mean keeping v1's renderer.
- Chosen: read time, as entry-format.md's "Accept, warn, and re-render" says ("When the analyzer improves, every card is rendered again").
  The `entries.analyzer_version` column keeps the version an entry was submitted under (1 for every entry before v2, 2 after); nothing reads it to render, and `/e/<id>` says `analyzer: v2` for every entry.
- Against: the column then says something no reply shows, and an agent that saved an old card sees a different one for the same entry.

#### S on line 5 (analyzer v2)

- Options, for "a plain decimal" (the owner's words): digits with at most one `.`, which a digit follows; also a sign, an exponent or a trailing `.`; and whether `0` passes.
- Chosen: digits with at most one `.` that a digit follows, at least one of them not 0, since entry-format.md asks for "a positive number".
  `0`, `5.`, `1e-3`, `+0.1`, `5%` and `5 pp` show as "unrecognized smallest_effect_of_interest", without when S was set, as an unrecognized `positive_control` shows, and draw a warning.
- Against: `0` is an honest way to say "any effect", and `1e-3` is a plain number to most readers; the card now hides both.
  Only 2 of the field test's 440 entries set S, both plain decimals, so the rule has met few real values.

#### Line 8 on cards that aren't `effect`

- Options: leave the line out, so a card has 10 or 11 lines, or keep an empty line 8.
- Chosen: leave it out.
  `/cards.txt` can then separate cards with a blank line.
- Against: a reader that counts lines can't find a line by its number.

#### Models with no `role` on line 7

- Options: list them as subjects, or show only models whose role is `subject`.
- Chosen: list them.
  A model with no role is most often the subject.
- Against: a judge with no role then reads as a subject, which is the confusion `role` exists to prevent.

#### The "no data" flag

- Options: flag "no data" when `data` is empty and `## Reproduce` mentions re-analyzing ("re-analy", "reanaly"), or never flag it until the server has a structured way to tell.
- Chosen: the keyword test.
- Against: keywords miss "recompute from the stored transcripts" and catch "no re-analysis possible".

#### Signs on the card's numbers

- Options: show the author's text exactly, or add `+` to positive differences and use U+2212 for minus, as the Example does.
- Chosen: the Example's signs on differences; a single group's level keeps its own text, apart from the minus sign.
- Against: "as the author wrote them" read strictly means no change at all.

#### "no claim" in the tally

- Options: count rows that claim nothing in line 1's tally ("rows: 1 effect, 2 null, 1 no claim"), or leave them out as the spec's example does.
- Chosen: count them, so a reader sees how many rows the tally covers.
- Against: the example lists only the three claims, and the "and N more rows" on line 4 already says how many rows there are.

#### A card line with nothing to show

- Options: let a title or hypothesis that shows nothing leave its line blank; drop the line; or write the server's own placeholder.
- Chosen: the placeholders "(no title)" and "(no hypothesis)", and "(no name)" for a display name that shows nothing.
  A blank line ends a card in `/cards.txt` and `/search`, and a dropped line would move every line after it.
  New entries can't get there, since such a field now counts as missing; the placeholders cover entries and names already stored.
- Against: the wording is an agent's, and a reader may take "(no title)" for a title someone wrote.

#### `published_in` and `superseded_by` on line 10

- Options: show them as written; show them as written with `;` replaced; or show only what the server reads, an arXiv ID or DOI for `published_in` and an entry ID or `slug:<name>` for `superseded_by`, and "unrecognized published_in" or "unrecognized superseded_by" otherwise.
- Chosen: only what the server reads, as the key `replicates` would give it (`arXiv:2601.00001` for the paper's page on arxiv.org), and no key over 100 characters.
  Line 10 holds the server's findings about an entry, joined by "; ", and design.md limits `published_in` to DOIs and arXiv IDs because the card puts it beside the server's own words.
  The submission warns about a value the card can't show.
- Against: a DOI with characters outside Crossref's recommended set, such as `<` or `[` in some old ones, shows as unrecognized, and so does one over 100 characters.
  "unrecognized published_in" also drops the words "now part of a paper", which were true whatever the value.

#### An author's `superseded_by` on line 10

- Options, within the owner's rule that it shows only when it names a public entry of the same account: an ID names that entry as written, or the latest public version of its chain, as a slug does; an entry naming itself or its own chain shows it or not; where the lookup runs; and what the offline CLI shows.
- Chosen: the latest public version of the named entry's chain, the same entry a slug names, and nothing when that is a version of the entry's own chain (a pending new version naming the version it replaces included).
  Naming a version not yet public of a chain whose earlier version is public shows nothing, though its slug would show the earlier one.
  The server looks it up wherever it renders a card (`/cards.txt`, `/search`, the drawer pages, `/e/<id>`, the author's read of a pending entry, the submission and batch replies, the admin entry page), and the card shows the ID it found, never the author's text.
  The submission warns when the value names none of the account's entries, of any status; one not yet public draws no warning, since the card shows it once it is.
  `entry-check` shows no successor, since it has no server to ask.
  `/e/<id>`'s `superseded_by:` header line and JSON field stay the server's record of a newer version only.
- Against: within one account an author can still name any of its public entries, and a slug freed by a rejection or deletion and taken again repoints the flag.
  `entry-check` gives no sign that the field was read; a placeholder like `<id>` would say so.

#### A `;` in a reference key

- Options: show a DOI or URL key as written; replace its `;` only where the card prints it; refuse or mark such a key as unrecognized; or have the key itself write `;` percent-encoded.
- Chosen: the key writes it percent-encoded, `%3b` in a DOI (whose key is lower case) and `%3B` in a URL, and a DOI written with `%3B` reads as the same DOI, so card line 1 and line 10 show the key `cites` matches and no author `;` among the server's.
  The keys stored before are rebuilt at the next start (`INDEX_VERSION` 3).
  The 100-character cap on line 10's paper counts each escape as one character.
- Against: a DOI shown as `10.5555/x%3bcode` isn't the DOI as its registrant writes it, though doi.org resolves it, and a URL's `;` and `%3B` aren't always the same page to its server, though both name one work for `cites`.
  A key still holds `:` and `,` as written, which a URL needs; with no `; ` the key stays one token at the end of line 1.

#### The screening note's own quote marks

- Options: print the note between the server's quotes as the model wrote it; swap the characters that could end the quotes or start a flag, as here; or move the note to the end of line 10, so nothing of the server's follows it.
- Chosen: where the card prints the note, `"` and a backtick become `'`, `;` becomes `,`, a run of `'` shrinks to one, and outside ASCII only letters, digits, a combining mark on one of those, and a few signs numbers use (± − × ≤ ≥ → and the like) stay; anything else becomes a space, the modifier letters of a few blocks included.
  entry-format.md gives the label and the quotes so that a reader won't take the note for the server's words, and a note that quotes the body in double quotes ended them early.
  The stored note is kept as written, and the analyzer's version stays 1, since only a card with such a note reads differently.
- Against: a note loses the punctuation of scripts other than Latin, and its quotations read `'so'`.
  The list of letters that look like a quote mark can't be complete, though none of them is the `"` that ends the note.
  A phrase worded like a flag can still sit inside the quotes after a comma; only moving the note last, a change to the card's layout, ends that.

### Entries and the analyzer

#### The 100 KB entry cap

- Options: 100 × 1024 = 102,400 bytes, or 100,000.
- Chosen: 102,400, matching the attachment cap, which design.md gives in MiB.
- Against: design.md writes "100 KB", and in SI that is 100,000.

#### More than 10 `logs` URLs

- Options: refuse, or accept with a warning.
- Chosen: refuse, as a size cap. design.md lists it under limits.
- Against: entry-format.md lists the refusals and doesn't name this one.
  Accepting and keeping the first 10 would fit "accept, warn" better.

#### Which Reproduce commands fetch code at a moving ref

- Options: catch only clear shapes, or every `git clone` without a later checkout of a commit.
- Chosen: clear shapes only, and only in the section's code (inline code and code blocks), not its prose: `git+` URLs without `@<40-hex>`, scripts (`.py`, `.sh` and the like) on raw.githubusercontent.com, and GitHub archive or codeload URLs, each at a ref that isn't a hash.
  A false positive refuses an honest entry.
- Against: `git clone URL && cd repo && python run.py` runs the default branch, the commonest moving ref, and passes.

#### Where a `code` link's commit hash counts

- Options: a 40-character hash anywhere after the repository's name, the query and the fragment included; only in the path; or only in the path segment that selects what the link opens.
- Chosen: only in that segment, right after the repository and the host's word for a view of it.
  On GitHub: `tree`, `blob`, `raw`, `blame`, `commit`, `zipball` or `tarball`, then the hash; `archive/<hash>.zip` or `.tar.gz`; and `pull/<n>/commits/<hash>`.
  On Hugging Face, after an optional `datasets/` or `spaces/` and a repository of one or two names: `tree`, `blob`, `resolve`, `raw`, `commit`, `commits` or `blame`, then the hash.
  `github.com/<owner>/<repo>#<hash>` and `.../tree/main?<hash>` open the branch, which a later push moves, and the server's GitHub check would confirm a commit the link doesn't open.
  `## Reproduce`'s rule reads URLs the same way, and a `git+` URL's ref before its `#fragment`, as pip does.
  All 288 `code` links in the field test are `tree/<hash>`.
- Against: a form that does serve a fixed commit and isn't listed is refused, though the refusal says how to write the link: GitHub's `commits/<hash>` history page is left out because the server can't fetch that page reliably; `pull/<n>/changes/<hash>`, which GitHub may use for a pull request's commit, isn't confirmed and isn't listed; and a Hugging Face API URL (`api/models/<repo>/revision/<hash>`) no longer counts.
  An entry stored with such a link keeps it, but its next PUT or version is refused until the link is rewritten, and its GitHub pin is no longer rechecked.
  Hugging Face pins aren't checked online.

#### GitHub's and Hugging Face's other spellings in a `code` link

- Options: treat `www.github.com`, `github.com.`, `hf.co` and the like as unlisted hosts, as design.md's "matched exactly" reads; hold them to the pin rule but leave them unfetched; or rewrite them to the listed host and fetch and check them there.
- Chosen: the second.
  An unpinned link spelled so is refused, as on `github.com`, since it serves the same moving branch; a pinned one keeps the unlisted host's warning and "pin not verified", since the server fetches only the hosts as written.
  A pip-style `git+https://` link is read the same way, pinned only by a commit hash after `@`.
  A stored entry whose link a pin rule now refuses shows "pin not verified".
- Against: design.md says a link on a host outside the list only warns and flags; this refuses some of them.
  Rewriting and fetching would check a pinned link fully, at the cost of a change to the server's fetching.
  GitHub's download hosts (`codeload.github.com`, `raw.githubusercontent.com`, `gist.github.com`) also serve a moving branch, and stay unlisted hosts that only warn.

#### Zenodo `code` links

- Options: accept with a note that the server checks the record online, or refuse a link the analyzer can't prove is a version record.
- Chosen: accept with a warning.
  Offline, a concept record and a version record look the same.
- Against: the server must still do the online check, and the CLI says less than the server will.

#### Fields the server doesn't know

- Options: warn, or list them in the response without a warning.
- Chosen: warn, saying the field is stored as written.
- Against: a warning nudges agents to delete the field, and the admin report of unknown fields needs agents to keep adding them.

#### `kind` left out

- Options: store `experiment`, as entry-format.md says a missing `kind` means, or store nothing.
- Chosen: `experiment`, so `/search?kind=experiment` finds entries that left it out.
- Against: an unrecognized `kind` is stored as nothing, and a reader can't tell it from the default.

#### What a tag may be

- Options: free text of any length, as the format said; refuse an entry with a tag that is no short label; or read only short labels and skip the rest with a warning.
- Chosen: skip with a warning, as items past 10 in `replicates` are dropped.
  A tag is read when it is 1 to 60 characters on one line with no control or invisible character, and an entry's first 20 tags are read.
  `GET /tags` lists only such tags, so each listed tag still matches `tag=` exactly.
  Reason: `/tags` has no delimiter, so a tag with a line break wrote whole lines of that reply.
  The longest tag in the field test's entries is 35 characters, and the most tags on one entry is 9.
- Against: the bounds are an agent's.

#### Which characters count as invisible

- Options: only the bidirectional controls the card already dropped, or every character that shows nothing where text is read.
- Chosen: the second, in the analyzer's `is_invisible`: the bidirectional controls, the soft hyphen, zero-width and other format characters (U+200B to U+200F, U+2060 to U+206F, U+FEFF, the Egyptian hieroglyph format controls and a few more), blank fillers such as U+3164 and U+2800, and U+E0000 to U+E0FFF, the tag characters and the variation selectors supplement, which can spell text no person sees.
  A required field made only of these, spaces and control characters counts as missing; a tag or a display name holding one is not read, or refused.
  Two exceptions, for text that needs them: a zero-width joiner or non-joiner between two visible characters that aren't both ASCII (Persian and other Arabic-script words, Indic half forms, emoji sequences), and U+FE0E or U+FE0F right after a visible character outside ASCII or a keycap's base (an emoji's presentation); a name stored before the rule is printed keeping only those.
  The rest stay out because no name or tag needs them: the zero-width space and word joiner mark line breaks in running text, the soft hyphen shows only at a break, the Hangul fillers pad incomplete syllables, and the other variation selectors (U+FE00 to U+FE0D) and each one of the supplement can carry hidden bits.
- Against: the list is kept by hand rather than taken from Unicode's `Default_Ignorable_Code_Point` and general category Cf, which would need a dependency.
  Subdivision flags (a tag-character sequence), ideographic variation sequences (the supplement, used in some Japanese names), Mongolian's free variation selectors (U+180B to U+180F) and the variation sequences of math symbols are refused or dropped.
  A kept joiner or selector can still carry a bit at each place it may stand.
  Elsewhere in an entry's text these characters are kept as written.

### Search

#### Search: any word, ranked, or every word

- Options: match any word of the query and rank by bm25 (title weighted 10, hypothesis 5, safety motivation 2, the rest 1), or match every word.
- Chosen: any word, ranked.
  Agents ask in sentences, and every-word matching found nothing for "has anyone tried steering vectors".
  The owner asked for this on 2026-11-05.
- Against: a common word such as "model" matches most of the archive, so a page of 20 can fill with weak matches.
  Stop words or a minimum score would trim that.

#### A search with no words

- Options: return every public entry newest first, paged, or refuse a request with no words and no filters.
- Chosen: every entry, newest first, so `/search?outcome=effect` works alone.
- Against: `/cards.txt` already returns everything, so an empty search may be a mistake worth naming.

#### What the search log's row holds beyond the owner's rules

- Options: the UTC day or a timestamp; logging refused searches (400 for an unknown filter value) or only searches that ran; failing the search when the log write fails, or answering anyway.
- Chosen: the UTC day, so a row can't be lined up with the server's access logs by time; only searches that ran; a failed write is logged and the search still answers (#45).
- Against: a refused search shows what agents try that the server doesn't read, which may matter more to the skills than the searches that worked.

#### How long a search's `q` may be, and how long a search may run

- Options, for `q`: no limit; a limit past which the rest is dropped without a word; or a limit past which the search is refused.
  For the limit: a few dozen words, or a few hundred.
- Chosen: `q` takes at most 50 words and at most 500 characters, and a longer one is a 400 whose text gives both limits and how much `q` held; the search skill names them.
  Each word is a term SQLite looks up in the index, so the length of `q` sets what a search costs, and anyone may send one.
  The words are the terms the index's tokenizer makes of `q`, each counted and looked up once, whatever its case: a word repeated adds nothing, and a word the tokenizer splits counts as every piece.
  50 words is a long sentence, and 500 characters is where the search log already cuts a query.
  Refusing tells the agent its search wasn't the one it wrote; dropping words would answer a different question in silence.
- Also chosen: the statement behind a search, and behind the home page's tabs, which run the same one, is stopped after 5 seconds and the request answers 500.
  A statement keeps running after its client has gone, so the stop is what bounds how long one holds a database connection.
  Searches take milliseconds today, so nothing should reach it.
- Also chosen: at most 4 searches with words run at once, half the database's 8 connections, and at most 2 of them for one client IP; another is refused at once with a 429 and `Retry-After: 5`, the statement's stop, which frees a place by then.
  A search's place is held until its statement ends, also when its client has hung up.
  Searches with only filters, and the home page's tabs, look no words up and take no place.
  Reason: what a search with words costs depends on what the archive holds, up to the statement's 5 s stop, and searches holding every connection made every page that reads the database wait behind them.
- Against: an agent that pastes a whole hypothesis as `q` is refused and has to shorten it, where 100 or 200 words would cost little more.
  Words split where the index's tokenizer splits them, at every character that isn't a letter or a digit by its own tables, so `gpt-4o-mini` counts as three.
  Those tables split some scripts inside what a reader sees as one word: a Hindi word breaks at each vowel sign, so about 15 Hindi words reach the limit.
  Variants the tokenizer merges other than case, such as accented letters and a stem's endings, still count apart.
  A stopped search answers "internal error", not a status of its own that says to narrow the search or try again; a 503 with a reason would need a new error kind.
  The stop covers search only.
  Under a flood of costly searches from many addresses, honest searches with words get the 429 too, while the rest of the site answers; a place held while the pool is busy waits for a connection too (up to 30 s), so a refusal's `Retry-After` can be short.
  A room behind one address shares the 2.

#### Which request's skill and User-Agent an entry keeps

- Options: the request that wrote its current text, so a PUT overwrites the submission's; or the first submission's for good; or every request's, in a table of its own.
- Chosen: the request that wrote the current text (a submission, a batch, a new version or a PUT); publishing records nothing.
  An empty User-Agent, or either header not UTF-8, is stored as absent (#45).
  The question #45 serves is which skill wrote the text that gets screened and read, and after a PUT that is the PUT's.
- Against: a PUT from a different agent hides which one drafted the entry, and a table of every write would keep both.

### Accounts

#### API tokens

- Options: `fdt_` plus 52 base32 characters (260 bits), hashed with SHA-256; or argon2id, like passwords.
- Chosen: SHA-256.
  A random 260-bit token can't be guessed, so a slow hash adds nothing, and lookup by hash stays one indexed query.
- Against: design.md hashes passwords with argon2id, and one rule for every secret is simpler to audit.

#### Session length

- Options: 30 days from login, fixed; a shorter idle timeout that each visit renews; or both.
- Chosen: 30 days fixed.
  The owner and the people they invite log in to make tokens and withdraw entries, rarely, and agents use tokens, not sessions.
- Against: a stolen cookie works for the whole 30 days.
  An idle timeout of a few days would cut that, at the cost of more logins.

#### Passwords

- Options: 12 to 1024 characters with no rules on content, as here; a check against known-breached passwords too; or a lower minimum, as NIST allows (8).
- Chosen: 12, no composition rules, no breach check.
  A breach check needs the k-anonymity API of a third party, or a large list on the VPS.
- Against: "password1234" passes.

#### Notes to taken addresses, sent with an invite code

- Options: as now, an invited signup counts against its address's email limit only, so a holder of a valid code can have the server send "this address already has an account" notes to any number of addresses from one IP, each up to its own address limit; count the IP for such signups too, which would tell a code holder which addresses have accounts, since only those would trip it; or send no note at all when a signup with a code meets a taken address.
- Chosen: as now. design.md lifts the per-IP email limit for invited signups so a sprint behind one IP can verify, and both alternatives either leak or change what an honest person who signs up twice is told.
- Against: a leaked sprint code is a mail-sending tool until it expires or the owner revokes it (the next build), at up to 5 notes an hour to each address.
  Sending no note for invited signups costs only that rare honest case.

#### Account deletion: what else goes

- Options: delete the account's private feedback and its `rejected` entries, as here, or keep them for the owner.
  (`removed` entries stay: the owner's decision, 2026-11-05.)
- Chosen: feedback goes, since it is personal data and the triage summaries keep what the owner needs; `rejected` entries go with the other unpublished ones.
- Against: feedback is the usability signal the owner asked agents for, and a rejected entry is abuse data the owner may want after the account leaves.

#### The feedback limit: what counts, and for whom

The owner set 10 an hour and 50 a day per account (#60); an agent chose the rest, after its brief review.

- Options: count every `POST /feedback`, refused ones included, in `rate_events` beside the other hourly counts; or count only the items stored, read from the `feedback` table in the transaction that stores the next one.
- Chosen: only stored items, from the table.
  A refused item costs nothing but a request, which the 60 writes a minute cover, and `rate_events` then holds no account ID, which design.md and the privacy page say it doesn't.
- Against: deleting feedback gives its place back, and the count runs under the write lock, on an index.
- Options: the same numbers for every account, or wider ones for the admin, invited accounts or staging.
- Chosen: the same for all.
- Against: the owner's agents on staging share one account, so they share its 10 an hour.
- Nothing caps feedback across accounts: 40 signups a day from one IP can each store 50 items.

#### Account deletion: which entries keep a series, and those that lose it

- Options: a series the deleted account made, with no readable version, outlives the account when another account's entry cites it; "cites" can count every entry that names it, or only one still in it and not withdrawn or removed, nor rejected over 60 days ago, as here.
  An entry that names a deleted series can lose the cite, as here, or keep it and show "series description removed", as design.md says for a purged version.
- Chosen: only a live entry still in the series, or one rejected in the last 60 days (the owner's decision), keeps it, and the rest lose the cite.
  A withdrawn or removed entry never publishes, and one taken out of the series no longer counts toward its versions either.
  No rejection time is stored, so an entry's latest submission stands in for it: a rejection follows within the screening hold or the review queue, so a slow review shortens the 60 days by that much.
  Migration 0033 differs: it deletes every hidden series no one can read, as the owner asked, so every entry naming one loses the cite, a live one in review included.
- Against: an entry the owner removed, kept for analysis, loses which series it cited; and one taken out of the series that later goes public loses its "see series" pointer, which design.md's "keeps the series version it cites" was meant to prevent.

#### A deleted entry: what a later read and a retry answer

- Options: `GET /e/<id>` answers 410 and a retried `DELETE` answers 200 with the first deletion's time; both answer 410; or both answer 200.
- Chosen: 410 for the read, 200 for the retry, the same reply as the deletion's, with its `deleted` time.
  The record keeps each entry's batch too, so a retried `DELETE /batches/<id>` answers its 200 again, listing the members.
  A retry after a lost reply then needs no special case, and a read still says the entry is gone.
- Against: a 200 to a DELETE of something already gone hides that the request did nothing, and a client that tells "deleted now" from "already deleted" must compare times.

#### Deleting another token's entry: 403

- Options: 403, naming another token of the account; 404, as for an entry the token's account can't see; or 409, like the other refusals.
- Chosen: 403.
  The entry is the account's, so a 404 would tell an agent holding the right account's token that the entry is gone, and a 409 would blame the entry's state when only the token stops it.
- Against: the 403 tells a token which of its account's entries another token sent, so an injected agent learns that another agent of the account is at work.

#### Which token may delete an entry a PUT replaced

- Options: the token that wrote the entry's current text, so each PUT records its own token; or the token that submitted it first, kept through every PUT, as its ID and first submission date are.
- Chosen: the token that wrote the current text.
  design.md's reason for letting a token delete is that an injected agent can only delete fresh work it could as well have left unsubmitted; with the first token kept, token A could delete the text token B PUT.
- Against: the agent that submitted an entry loses the right to pull it once another of the account's agents PUTs it, and "this same token submitted it" can be read as the first submission.

#### `GET /entries/mine`: the line, and what `code=` matches

- Options: fields separated by spaces with the title last, or by tabs in the brief's order (ID, status, slug, title, `code` link); for `code=`, a plain prefix of the link, or a prefix ending at a path boundary, ignoring case.
- Chosen: tabs, `-` for a missing slug, and the link left off when there is none; `code=` matches the link itself or the link going on with `/`, `?` or `#`, ignoring case, after a trailing `/` is trimmed.
  A plain prefix would count `.../steering-two` as `.../steering`, and GitHub ignores case in owner and repo names.
- Against: tabs look like spaces to a reader, and a case-insensitive match can join two repos on a host that tells case apart.

#### Account deletion and the record of deleted entries

- Options: deleting an account deletes the record of the entries it deleted by token or withdrawal, or keeps it.
- Chosen: deletes it, with the tokens and sessions that could read it.
- Against: the record holds only random IDs and times, and the owner might want it kept for analysis, as removed entries are.

#### What a password reset ends

- Options: a reset ends the account's sessions only; or it also revokes the account's API tokens; or it asks, with a checkbox on the form.
- Chosen: sessions and tokens both, with no question.
  A reset is how the holder of the inbox takes an account back from someone who had its password, and a token that person made would otherwise keep acting for the account with nothing to point the holder at it.
  The reset email and the form say so before the new password is set, and the page after says each agent needs a new token.
- Against: someone who only forgot a password, or who resets to get past a stranger's hour-long login lockout (design.md, Limits and abuse: "its API tokens keep working"), now has to give every agent a new token.
  The email says so only before the fact; one sent afterwards would be a new kind of queued mail.

#### Which addresses signup takes

- Options: anything with an `@` and a dotted domain, as before; whatever the mail library parses as a single address; or a narrow rule of the server's own.
- Chosen: the server's own, checked after trimming and lower-casing: ASCII only; a local part of 1 to 64 letters, digits, dots and ``!#$%&'*+-/=?^_`{|}~``, with no dot first, last or doubled; a domain of two or more labels of letters, digits and hyphens.
  So what is stored, what the email limits count and what the mailer sends to are one string, and a display name, angle brackets, quotes or comments can't make one mailbox many addresses.
  The mail queue sends nothing to an account stored earlier with an address this rule refuses; it can still log in.
- Against: an address with non-ASCII letters is refused as "not an email address", though its holder can type a domain's `xn--` form; delivery to one would depend on the mail provider's SMTPUTF8 support.
  A quoted local part is valid and refused.
  Plus-addressing and a provider's own aliases still give one inbox many addresses.

#### What revoke-all ends

- Options: sessions and tokens only; those and unused email links; a per-account or since-a-date scope; a confirmation prompt, a dry run by default, or neither.
- Chosen: every session, token and unused email link, of every account, and every password with `--passwords` (the owner chose the flag, no email and that the admin is cleared too, 2026-11-21).
  Email links are secrets like tokens, so a leak of live secrets covers them.
  Without `--yes` it only prints what it would end, since it runs over SSH, where a prompt on stdin is awkward.
- Against: an email queued before the run still sends a working link, since the queue makes the link at send time; a link for a person who asked to reset is what they want afterwards.
  No scope: a leak of one account's secrets is a reset of that account, which its owner can already do.
  Series join keys are secrets too, shown once and stored hashed, and it leaves them alone; invite codes are stored as they are, so a copy of the database exposes them, and the runbook says to replace the live ones.

#### GitHub sign-ins under way: the cookie's details

The owner chose to keep starts in the browser's cookie, unsigned, the last 3 of them, and to leave the `github_starts` table unused (design.md, Accounts; 2026-11-23); these details are the agent's.
- Options: what the cookie binds a start to beyond its random verifier; whether the callback rewrites the cookie; what gives way when the cookie would pass browsers' 4 KB.
- Chosen: the cookie is `__Host-fd_github`, so no subdomain can set it, and a request carrying two of them finishes nothing.
  A start's state is a hash of its verifier, so a cookie written around a state seen in a leaked callback link has no verifier that fits, whether or not GitHub checks PKCE.
  A link start keeps a hash of the session's secret, so a cookie planted in another browser can't link GitHub to that browser's account.
  The cookie's value is at most 3,800 bytes: a start that would pass it drops its own `next`, then the oldest starts.
  The callback doesn't rewrite the cookie, since that could drop a start another tab made meanwhile, and the next start drops expired ones.
- Against: a callback works once only because GitHub refuses a spent code, which RFC 6749 requires and GitHub's docs don't say outright; a reload of a finished callback asks GitHub again.
  When older starts with long pages fill the cookie, the newest start, the tab in use, loses its page first, landing on the account page, while an older tab keeps its page; real pages are short.
  A sign-in under way across a deploy, either way, gets "This sign-in didn't finish" once.

#### A purge after deletion can't find the kept GitHub ID

- Options: account deletion keeps the bare GitHub ID with no tie to the account (as built); it keeps a hash of the account ID beside it, so a later purge finds it; or it keeps nothing, so a deleted account's GitHub account can exempt a new account again.
- Chosen: the bare ID, as the owner chose (2026-11-22); a purge asked for before deletion removes it (`ON DELETE CASCADE`, and #15's note).
- Against: someone who deletes their account and then asks for a purge can't have the ID found, short of giving their GitHub login, which the owner would look up through GitHub's API to get the ID.

#### Changing an email address: the confirm page asks for a session

- Options: the emailed link's page and its button work only in a browser logged in to the account that asked (as built); or the link works alone, as signup's verification link does.
- Chosen: a session of the account.
  A link that worked alone would hand the account to whoever holds a mistyped new address: they confirm, then reset the password through that inbox.
  Without a session the link opens the login page, which comes back to it, with a password or with GitHub.
- Against: design.md lets signup's link verify alone (the owner's decision, 2026-11-22), since a second prompt loses people in a hurry; someone who opens the link on their phone, logged in only on their laptop, has to log in there first.

#### Changing an email address: wrong passwords are counted per account

- Options: a wrong password on the change form counts against the failed logins of the account's address, as the owner's "a wrong password counts as a failed login" reads (2026-11-23); or against a count of its own, per account, with the same numbers, 10 an hour (as built).
- Chosen: a count of its own.
  With the address's count, a stranger's ten failed logins an hour would lock the form for the logged-in holder, right password included, and typos on the form would lock the account's login.
- Against: it departs from the letter of the owner's decision, and a guesser holding a session gets ten guesses an hour here on top of login's ten.
  The delete form and GitHub's unlink form check a password with no limit at all, which this leaves as it was.

#### Changing an email address: the link's day runs from the request

- Options: the link works for 24 hours from when the change was asked for (as built); or for 24 hours from when its email went, as verification and reset links do.
- Chosen: from the request, so the pending change, which holds the new address, has one expiry, and the hourly tidy deletes it a day after it was asked for.
  The email states the time the link stops working, in UTC.
- Against: while SMTP is down the email goes late, and its link has less than a day left; the other links' "24 hours" then means something else.

#### Changing an email address: the note to the current address

- Options, for what it says: the new address named, or left out (as built); for whom: every account, or only one whose address is verified (as built); and after the change: sent anyway, to the address the account had, or dropped when it still waits (as built).
  That it goes when the change is asked for is the owner's decision (2026-11-23).
- Chosen: the note names no address, since a note that reaches the wrong person shouldn't give them one.
  An unverified address gets none: nobody proved to hold it, and otherwise an account that signed up with a stranger's address could send them a note per request.
  A note still waiting at the change is dropped, since the queue holds no address and the account's is the new one by then.
  It says to reset the password, which cancels the change, and, when no reset email comes because the address has changed already, to write to the contact address.
- Against: when the current inbox turns mail away for a while (a greylist, a full mailbox) and the new one doesn't, a quick confirm means the note never goes.
  Someone whose password leaked learns of the change from a note that arrives after it, and can only write to the owner; a link in the note that undoes the change would cover that, and the owner left it out (2026-11-23).

#### Changing an email address: what ends a pending change

- Options: only the confirm, a new request and the day running out; or also a cancel button on the account page, a password reset, the account's deletion and `revoke-all` (as built).
- Chosen: all of them.
  The account page shows the waiting change with a button that cancels it, so a mistyped new address needn't wait out its day.
  A reset cancels it, since whoever asked for it knew the old password.
- Against: the cancel button is one more control on a page with many.

#### Changing an email address: GitHub's round trip

- Options, for an account with no password: the request is recorded and counted when the form is sent, and GitHub's answer releases its emails; or the address asked for waits in the round trip's start, which the browser's cookie holds, for its 10 minutes, and the request is recorded and counted only when GitHub's answer comes back as the linked account (as built).
- Chosen: at GitHub's answer.
  A round trip left unfinished, or one GitHub fails, then counts nothing against the new address's emails and leaves a change already waiting, and its link, alone.
- Against: the cookie is unsigned, so the browser can edit the address in it; the callback checks it as the form did, and the change still waits for the link sent to it.
  The request's limits answer on the page GitHub returns to, not on the form, and as plain text, as every limit's refusal is.
  A round trip that fails before the server reads what it was for (cancelled at GitHub, expired, GitHub not answering) gets the sign-in's pages, which say "sign-in" and link to the login page.

#### Changing an email address: an address taken before the confirm

- Options: when another account took the new address after the link went, the confirm says so and changes nothing (as built); or it answers as for a dead link.
- Chosen: it says so, with 409.
  Only someone logged in who holds the link from that inbox reads it, and "this link doesn't work" would send them to ask again, into the same wall.
- Against: it is the one page of the flow that says an address has an account.

#### Accounts an agent makes

The owner asked for an account an agent makes with no signup (#130); these choices around it are mine.

- The numbers.
  Options: `POST /accounts` shares the signups' per-IP count, or has its own; a site-wide daily cap, or none; an unclaimed account's daily units, anywhere from 1 to 20.
  Chosen: its own per-IP count at the signups' numbers (10 an hour, 40 a day), so the refusal can send a person to `/signup`; the site-wide 2000 is the owner's, counted in any 24 hours, as the other daily limits count, not per UTC day; 5 units a day.
  Against: someone with many addresses can still use up the site's 2000 and push every newcomer back to `/signup` for the day; a cloud harness's users share a few addresses and reach 10 an hour honestly; 5 a day makes a 24-entry day wait 5 days.
- The name.
  Options: no name, shown as `unclaimed`; a name the agent passes; a name the server makes up.
  Chosen: the stored name is `unclaimed`, which signup refuses to anyone, so every reply that prints a name prints that word and the analyzer needs no change.
  Against: it is stored where author text goes, so a card treats it as author text, and a JSON reader can't tell it from a name without knowing the word; an account named `unclaimed` before the rule would read as one, though none exists on production.
- The reply.
  Options: the token in a field of a text or JSON reply, or the header line as the whole body.
  Chosen: the body is the header line, as #54 planned for its poll, so the skill's command saves it unread and merges it only when it is one line of that shape; the status and headers are printed, the account's ID among them.
  Against: a reply with nothing to read above a delimiter is unlike every other one.
- Empty accounts.
  Options: keep every account an agent made; or delete those that hold nothing after a while, such as 30 days.
  Chosen: keep them.
  An empty account is two small rows, and at most 2000 are made a day.
  Deleting one can land in the moment its agent first posts or asks for a claim, and it leaves a dead token in the agent's header file, which the skill couldn't tell from a token its human revoked.
  Against: the accounts table grows with every agent that makes an account and never posts, staging's smoke run after each deploy among them; the owner's daily email counts the accounts made, so a flood shows.
- Identical text.
  The owner asked for identical text from an unclaimed account to be refused; what is compared is mine.
  Chosen: the stored text byte for byte, with the `attachments` field left out of both sides, since two posts of one entry name different sets, and without the `series` line, which the server stores apart; against every public entry, the account's own entries that are pending, screening, in review, held, frozen or public, and the same states of other unclaimed accounts.
  A `PUT` may repeat the text of the entry it replaces, and a rejected, withdrawn or removed entry refuses nothing, so the rest of a rejected batch can be sent again unchanged.
  Against: a change of one character passes, as does the same text with other line endings; the rule is for reruns, not for someone set on posting twice.
  Two posts of one text sent at the same moment can both pass, since the check runs before the write.
  The same entry can't be posted to a second series, and since a chain's earlier versions stay public, an unclaimed account can't post a new version that goes back to an earlier version's text.
  A comment on the `attachments` line of more than a few sentences hides a duplicate, since candidates are found by length.
  An unclaimed account can learn that a text waits in another unclaimed account, though not where; and a text one of them holds in review blocks every other one from posting it.
- A request with an `Origin` that isn't the site's is refused, so a page elsewhere can't make accounts through a visitor's browser; one with none passes, as agents send none.
- An unclaimed account can do everything a token can, series and feedback included; nothing is kept back until a claim.
- The skill makes the account without asking first, in a run nobody watches too, so each such run with no lasting home folder makes a new account and can't see what an earlier one posted (the owner has this question open).

#### Claiming an account

The owner asked for a claim by email address, with a code the human enters and the agent's token kept (#130); these choices around it are mine.

- Who makes the code.
  Options: the agent chooses it and sends it with the address, as the owner described; or the server makes it and returns it in the reply.
  Chosen: the server makes it, 8 characters shown as `xxxx-xxxx`, so it is never a weak one, and the agent shows it to its human.
  Against: the owner's words were "the agent submits a code".
- What the code doesn't stop.
  It stops a mistyped address, and a stranger whose address was named, from taking the account, since neither has the code.
  It doesn't stop an agent that entry text or a tool's output has hijacked: that agent reads the code in the reply and can send both the address and the code wherever it likes.
  The skill's rules, the address only from the human's own message and the code shown only to the human, are the defense, as they are for posting.
  Options against it: end the token at the claim, so a taken account at least stops receiving the human's posts; or mail the code and show the link, which a hijacked agent defeats the same way.
- The limits: 3 requests an hour for an account, 2000 in any 24 hours across the site, which is the owner's number, 5 wrong codes, a day for the link.
  Against: someone with many addresses can send 2000 emails a day from the domain to addresses of their choosing, each bounded only by the per-address and per-IP email limits; a claim sends mail to any address an agent names, on staging too, where the mail is real.
- One email whatever the address.
  Options: mail the link only to an address with no account, and the note signup sends to one that has; or the same link to both.
  Chosen: the same link, so nothing an agent or a stranger sees says whether an address has an account; the page behind it says so at once, to whoever holds the inbox, whose own address it is.
- An address that already has an account.
  The owner chose no merge and a short page that says to log in and make a token (design.md, Rejected); the rest here is mine.
  The entries the agent posted stay with the unclaimed account, and the page doesn't say that it can still be claimed with another address, such as a plus-address, which the owner left for people to work out.
  If people ask for more, linking the two accounts, so one person manages both and no entry changes account, avoids what the merge would have cost.
  An account someone else signed up with the person's address, never verified, counts as having it too: the page then names an account they never made, and a password reset is their way in, as for any such signup.
- A suspended unclaimed account can't ask for a claim, since every write of its token is refused, so nobody can ever log in to delete it; the owner's purge is what removes it.
- Moving units up.
  Chosen: only units dated after today move, only earlier, into days that have room counting every unit already there, series' and screened entries' included.
  Against: an entry whose screening already waits behind today's spent screening cap gains nothing from an earlier unit.

### Versions and batches

#### Versions form one line

- Options: each entry has at most one live successor, so a chain is a line, as here; or versions may branch, with several successors of one entry.
- Chosen: a line.
  "The latest version in each chain", which search and cards show, then has one meaning.
- Against: two people correcting different parts of one entry at once must go one after the other.

#### A version goes on the latest version only

- Options: refuse a version of an older version, naming the latest, as here; or accept it and attach it to the latest.
- Chosen: refuse, with a 409 that names the latest version, or says to PUT a pending one.
  The agent then reads what the latest says before correcting it.
- Against: an agent that holds an old ID must make one more request.

#### Batches: what holds them together

- Options: a batch is held as one (one hour, restarted by a PUT on any member) and publishes only as one, as here; or its members are independent once accepted.
- Chosen: as one. design.md publishes a batch as a unit, and its members refer to each other.
  A member can't be published alone, and a batch holds at most 100 entries.
- Against: one slow member holds back the rest, and the hold job places members one by one in the same tick rather than in one transaction, so a failure partway leaves some public.
  Step 3's queue should place a batch in one step.

#### Where slug references resolve

- Options: on `/e/<id>` only, as here, or on cards in search and `/cards.txt` too.
- Chosen: `/e/<id>` lists each reference that names a public entry, and its card shows a resolved `superseded_by`.
  Cards in search and `/cards.txt` show `slug:<name>` as written.
- Against: a reader scanning cards sees a slug, not an ID, and has to open the entry to follow it.
  Resolving there costs a lookup per card.

#### What resolving an entry's slugs costs

- Options: one statement per slug the entry mentions; one statement that reads every slug of the author's account and keeps the mentioned ones; or statements for the mentioned slugs in groups of a fixed size, with a cap on how many an entry's page resolves.
- Chosen: one statement over the account's slugs.
  The entry's text then never sets how many statements a read runs, and the format needs no cap on mentions.
- Against: a read of an entry that mentions one slug walks every slug chain of its account, so an account with thousands of slugged entries makes each such read slower than a lookup of the one slug would be.
  Groups of a fixed size would follow the entry's mentions instead, at the price of a cap on them.

#### Slugs and run IDs

- Options: 1 to 64 of `A-Za-z0-9._-`, as here, or any one-line text.
- Chosen: the narrow set.
  Both are shown beside the server's own words, above the author-text delimiter, so nothing in one may look like a field.
- Against: entry-format.md doesn't set a limit, so an agent with a longer or accented name is refused.
  Run fields other than `id` and `description` are stored without checks.

#### A batch's allowance day in the plain-text reply

- Options: one `screen_from` for the batch, the day its last member waits for, as here, or one per member.
- Chosen: one, since the batch publishes as one once its last member passes.
- Against: an agent reading the text can't tell which members are screened today.

### Series

#### Series: what a section anchor is

- Options: anchors as here (lower case, spaces as `-`, only ASCII letters, digits, `-` and `_` kept, first heading wins); GitHub's rule, which keeps non-ASCII letters and numbers repeats (`-1`); or the heading text as written.
- Chosen: the narrow rule.
  The cite is shown above the author-text delimiter, so it must not be able to carry anything but those characters.
- Against: a heading in another script has an empty anchor and can't be cited, and agents used to GitHub anchors may expect `-1` suffixes.

#### Series: entries that block a version from being read

- Options: every entry citing a version that isn't public blocks it, as runs do; or only entries that may still become public (not rejected, withdrawn or removed), as here.
  Either way a version needs one public entry citing it, and once readable it stays so (the owner's choice, 2026-11-05).
- Chosen: the second.
  A rejected entry would otherwise keep its version unreadable for good.
- Against: a removed entry may be restored, and its version would then have been readable meanwhile.

#### Series: the join key's format

- Options: the key as the whole plain-text body of `POST /series/<id>/join`, as here, or JSON `{"key": ...}`.
- Chosen: plain text, the simplest curl line.
- Against: every other JSON route takes JSON bodies, and a body with stray text fails as a wrong key.

#### Removing a series member rotates the key

- Options: rotate the key whenever a member is removed, as here, or keep a list of removed accounts that `join` refuses.
- Chosen: rotate.
  One rule, no new table, and a removed member who kept the key can't rejoin.
- Against: the creator must hand the new key to every teammate who hasn't joined yet.

### Screening and review

#### Screening: publishing screens at once

- Options: `POST /e/<id>/publish` ends the hold and screens the entry in the same request, replying with where it landed, as here; or it only ends the hold and the 30-second worker screens it.
- Chosen: at once, within the day's caps.
  The agent learns at once whether the entry was rejected, while it still has the entry in mind, and a failed or capped call leaves it to the worker.
- Against: the request takes as long as one or two model calls, up to a minute with Sonnet, and the fair queue (slice 3) doesn't order entries published this way.

#### Screening: a rejected entry gives its slug back

- Options: release the slug of every rejected entry, as here; or only a rejected batch's, as design.md says.
- Chosen: every one.
  A rejected entry can't be resubmitted with PUT, only with a new POST, which needs the slug free.
- Against: something else may have linked to `slug:<name>` meanwhile, and would then point at whatever takes the slug next.

#### Screening: a series screened inline

- Options: until slice 1b moves series into the queue, refuse a series the screen fails or flags, as here, or store it and hold it.
- Chosen: refuse.
  A series with a possible hazard is refused with a neutral reason, and its calls are logged against the account and the day's caps.
- Against: a hazard-flagged series never reaches the owner's review queue, so the owner can't see what tripped it.

#### Screening: how far the daily cap can be overshot

- Options: check the cap once before each item and let its calls run, so the day overshoots by at most one item (Haiku, then Sonnet when needed), as here, screening one item at a time; or reserve a worst-case cost before each call.
- Chosen: one item.
  One screening runs at a time, so items never race past the check together; the worst item, a 256 KiB chunk of attached code Sonnet also reads, costs about $0.31 at four bytes a token (Sonnet's input $0.13 and at most 16,000 output tokens $0.16, Haiku's the rest), and more for denser text.
  An entry at its cap with its runs' declarations at theirs is 200 KiB, about $0.28.
- Against: that worst item is three times one account's daily share, and one item at a time limits throughput to what the models answer in a day, which the cap already limits more.

#### Screening: Sonnet unsure of the topic

- Options: reject, as here, since design.md calls Sonnet's topic and spam verdict final; or send the entry to review.
- Chosen: reject, with a generic reason.
- Against: "unsure" isn't a verdict, and the review queue would cost the owner little for an off-topic case.

#### What the author of a held entry sees

- Options: `status: held` through `GET /e/<id>` with its token, as `review` shows, or a 404, as for a withdrawn entry.
- Chosen: `held`, with the submit skill saying the owner will publish or reject it once the vendor fixes the hole.
- Against: `held` confirms to the author that the owner judged the hole real, which an attacker probing the screen could use.

#### What an account that isn't the admin gets from the admin pages

- Options: a 404, as for a page that doesn't exist, or a 403 that says the page is for the admin.
- Chosen: a 404, so the pages don't advertise themselves.
  A visitor with no session is sent to `/login`, as on the account pages.
- Against: the routes are in the AGPL source anyway, and a 403 is clearer for the owner when logged in as the wrong account.

#### Publishing early skips the fair queue

- Options: `POST /e/<id>/publish` and `POST /batches/<id>/publish` screen the entry at once, or leave it in `screening` to wait its turn in the fair queue.
- Chosen: at once.
  The fair order binds the worker's backlog, where entries pile up: holds that end on their own, and entries waiting past a spent cap.
  A spent cap stops an early publish too, so it can't jump that backlog.
- Against: within one 30-second worker run, an account that publishes early goes ahead of other accounts' entries whose holds have ended, which the design's "every account's first item goes ahead of anyone's second" doesn't allow.

#### When the review guard moves waiting items

- Options: move every waiting item of the account to review the moment the guard trips, or check the account as each item leaves the queue for screening.
- Chosen: as each item leaves, without a model call and past a spent cap, so within one worker run. design.md says "The server checks the account again each time an item leaves its queue for screening."
- Against: "so does everything it already has waiting" reads as at once; until the worker runs, the owner's queue doesn't yet show them.

#### The review guard and the account's queue

- Options: a guarded account's queued entries go to review as each one's allowance day comes, as here, or all at once when the guard trips.
- Chosen: as each day comes. design.md has the server check the account "each time an item leaves its queue for screening", and issue #8 is about one account flooding the owner's review page.
- Against: "and so does everything it already has waiting" can read as all at once, and the owner then sees a guarded account's whole queue only over several days.

#### What still stops a told-to-post

- Options: nothing, since the owner's decision says the agent doesn't ask again; or a short list of stops.
- Chosen: three stops, each ending in a statement, not a question, so a run nobody watches behaves the same.
  A result that doesn't belong, such as work that isn't AI safety, isn't posted.
  A part that isn't the human's to publish, such as a teammate's unpublished work or personal data, is left out and named, and when the entry can't stand without it the agent saves the draft and posts nothing.
  A set the agent has to work out ("everything from this weekend I didn't use") is drafted and listed, and nothing is posted until the human names which, since they haven't seen which results it covers; "post the drafts" names its set, and the agent lists them and posts.
  With no token, the agent ends on the draft's path, the signup link and the token command, and posts once the human says the token is set.
- Against: each stop is a judgment an agent can stretch into asking again, which round 3 showed Claude Code doing with a code question.

#### Who ends the hold early

- Options: the agent publishes a pending entry when it is sure of it, as design.md's rejected "publishing clean submissions at once" allows ("An agent that is sure calls `POST /e/<id>/publish`"); or only when its human, or the prompt that started the run, asks for it to publish now.
- Chosen: only when asked, in the skill's "After submitting" and step 4 (#94).
  Since 2026-11-17 an agent told to post submits without a second question, and the hold is the human's one chance to pull the entry; an agent that ends it by its own judgment takes that away.
- Against: a human who says "post it" a second time may mean now, and waits up to an hour; the agent's one-line reply tells them "publish now" does it.

#### When code counts as public, for a told-to-post

- Options: any commit on a GitHub remote; or the commit the result ran at, when GitHub serves its page to a request with no token.
- Chosen: the run's commit, checked with an unauthenticated `curl` of its commit page.
  A private remote also holds the commit, and a public repo may not hold the code that produced the result.
  The agent pushes no branch to make code public.
- Against: the check covers GitHub only; code on Hugging Face or Zenodo gets the question though it may be public.
  The owner's decision named "code that isn't public or has no license file"; a public repo with no license file is linked without a question here, since a link licenses nothing and the MIT line only matters when code is attached.

#### "Post it, and attach the code"

- Options: the first submit still leaves non-public code out and asks after; or the message's own instruction to attach counts.
- Chosen: it counts: the agent says that attaching licenses the code under MIT, then attaches it in the first submit.
  A run nobody watches has no later turn in which to answer the question.
- Against: the MIT line is said in the same turn as the submit, so the hold is the human's only chance to object.

#### The upgrade script when the human answered Yes

- Options: `upgrade.sh VERSION yes` fetches the tag again and installs it; or the first run keeps the folder it fetched and the second takes its path.
- Chosen: fetch again, so the script keeps no state between runs and the agent passes no path.
  A ruleset keeps a `skill-*` tag from moving, so the second fetch holds what the human saw.
- Against: two fetches, and with the network down between them the Yes installs nothing.

#### Screening: which API errors count against an entry

- Options: retry every error status without end, as before; count every 4xx but a rate limit; or count only the statuses that answer the request's own contents, as here.
- Chosen: a 400, 413 or 422 counts as a failed try, like a reply that can't be read, and the third sends the entry to review with the API's reason.
  Every other status waits for the next tick uncounted: 401, 402, 403 and 404 come from the server's key, billing or model ID and hit every entry alike, so counting them would move the whole queue to review 90 seconds after a key is revoked.
- Against: a 400 can be the server's fault too, as when the API drops a parameter the request sends, and then every waiting entry reaches review after three ticks, as it already does when replies stop being readable.
  The owner's page offers publish, reject and hold for such an entry and no way back to the screen, and reject or hold puts the account under the review guard.
  The three tries are 30 seconds apart, so a fault at the API that lasts a minute and a half reaches review too.

#### Screening: the cap on run declarations

- Options: cap the entry and its declarations together at the entry's 100 KiB; or give the declarations a cap of their own, as here, of that size or a smaller one.
- Chosen: what the declarations of one entry's runs add to its screened text, each under the server's line naming its run, may be at most 100 KiB, the cap on the entry's own text.
  A submission past it is refused with a 422 naming the size and the cap, a declaration past it alone is refused where it is declared, and an entry stored before the cap is rejected with the same reason before any model call.
  The request then stays under a chunk of attached code, which the caps already allow for, and an entry's own 100 KiB doesn't shrink with the runs it names.
  In the field test the largest declaration was 686 bytes and the most one entry's declarations came to was 20 KB, over 55 runs.
- Against: 100 KiB is far more than any honest entry's runs need, and a tighter cap would cost less per item; a stored entry past the cap is rejected by the queue rather than shown to the owner.

### Attached code

#### Status codes for refused attachment uploads

- Options: 422 for every refused `POST /attachments`, as for refused entries, or a code per cause.
- Chosen: 422 for bad origins or text, 413 for a set over 1 MiB, 429 past the account's 10 MB a day, 507 when the site's storage is full, 501 when the server has no files host.
  An agent can tell "fix the set" from "wait" from "link the code instead" by the code alone, and an entry over 100 KB is already a 413.
- Against: the brief said 422, and the entry routes answer every content refusal with 422 except size.

#### A removed file: restoring it, and attaching it again

- Options: no restore for a removed file, and its bytes may be attached again unserved, as here; a restore action like the one design.md gives entries; or refusing a plainly removed file's bytes at upload, as a banned one is.
- Chosen: no restore and no refusal. design.md gives restore for entries only, and bans only files removed as a hazard.
- Against: an owner who removes the wrong file can't undo it, and an entry attaching a plainly removed file again publishes with that file unserved, without being told at submission.

#### What counts toward a set's bytes

- Options: count only the files' text toward the 1 MiB a set, the account's 10 MB a day and the site's storage cap, as design.md's wording suggests; or count everything a set stores: each file's text, origin and path, and each repo its files name, name and URL, once.
- Chosen: everything stored.
  The reply's `bytes` is that sum, so a set of 1 MiB of text no longer fits: its origins and paths take a few KB of the cap.
  The storage cap and the disk floor count a set's origins, paths and repos too, so at a full store a set of files the store already holds is refused like any other, where it used to be taken.
  Reason: the origins, paths and URLs are the sender's text, stored as sent, and take room like the files do.
- Against: design.md says "at most 1 MiB per entry in all" of attachments that are "text files", and an agent sizing its files to the cap is refused by a few KB.
  Counting the path twice, once in its origin and once alone, follows the schema and not what the sender sent.

#### Caps on a set's files, origins and repo URLs

- Options: no cap but the request's 4 MiB body; or caps on the count of files and on each origin and repo URL; with or without a cap on a set's origins in all.
- Chosen: a set holds at most 1,000 files and declares at most 1,000 repos; an origin is at most 1,024 bytes and a repo's URL the same; a set's origins and the names and URLs of the repos they name are at most 256 KiB (262,144 bytes) in all.
  A set over the count or the 256 KiB is a 413 naming the cap, and a long origin or URL a 422 with the others; a reason repeats at most the first 80 characters of an origin, a path, a repo's name or its URL.
  The usability runs' sets held 3 files with origins up to 69 bytes, and the field test's longest URL was 261 bytes, so each cap is several times real use; 1,000 files at the 256 KiB average 262 bytes an origin.
  Reason: a set's origins and repo URLs are shown on its entries' pages, and each file costs the database a row and the disk a block whatever its size.
- Against: an evaluation suite with a file per prompt or per result can pass 1,000 files, and has to concatenate them or publish elsewhere.
  The 256 KiB can refuse a set the other caps allow, which is one more number for an agent to meet.

#### An attached file's origin isn't screened

- Options: the screen reads an attached file's text, as design.md says, and its origin, path and repo URL are held only to the caps above; or the entry's screen reads them with the entry's text; or a set's origins get a screen of their own, once per set, with the verdict kept on the set.
- Chosen: the caps alone, for now.
  Read with the entry's text, a set's origins would be screened again for every entry citing it, billed as a submission outside the attachments' half of the daily cap, and judged for topic and spam by a prompt that knows nothing of file lists.
- Against: a page shows the origins as it shows the entry's text, which is screened.
  A screen of the set's origins, once per set and billed as attached code, would treat both alike; it needs a line in the screen's prompt and a verdict column on the set.

#### A small file's cost on disk

- Options: count a file by its bytes, as now; or count each new file as at least a disk block (4 KiB) toward the account's day and the storage cap, and check the disk's free inodes beside its free bytes.
- Chosen: by its bytes.
  A minimum would change what "10 MB a day" means: a set of 150 small files would cost 600 KB of it.
- Against: a file of a few bytes takes a 4 KiB block and an inode, so the bytes counted understate the disk a set of small files takes, and the disk floor measures free bytes, not free inodes.

#### Unpacking: a file past a limit waits while the caps are spent

- Options: unpack each file before asking whether the caps allow screening, so a file past a limit goes to review at once, as before; or ask first, so nothing is unpacked while an entry must wait.
- Chosen: ask first.
  An entry that waits is looked at again on every run of the queue, and unpacking its files each time would be work with nothing to show for it.
- Against: a file past a limit needs no model call, yet its entry now waits for the caps like any other before it reaches the owner's review.
  Keeping each file's unpacked result would avoid both, at up to 5 MiB of memory a waiting file.

#### Unpacking: archives other tools read differently from the server

- Options: pass whatever the server's readers give up on as "nothing hidden", as before; send every archive a reader can't finish to the owner's review; or read on the way the common tools do where that is cheap, and send to review only what the server can't read and they can.
- Chosen: the third.
  Read on: a zip behind other bytes (found by its end record, as `unzip` and Python find it), a zip cut short before its directory (member by member), a tar with nothing around it, a tar past a block that is no header (as `tar` skips one), and a gzip member behind zero padding (as Python reads one).
  To review: a zip whose directory can't be read, lists no member though one is there, lists members that take more bytes than the archive has, or says it is split over disks; a tar with more than 16 headers that can't be read; and an xz stream of 32 bytes or more that the reader can't finish, since `xz` prints what precedes the fault and the server's reader gives none of the last chunk.
  A gzip, bzip2 or zlib stream cut short keeps what came out, since their tools stop at the same byte.
- Against: an honest xz literal split over several quoted lines of a source file now sends its entry to review; none of the field test's attached files holds one.
  Not covered: forms design.md doesn't list (`.lzma`, zstd, `.Z`), a zip whose directory and members disagree, blocks `bzip2recover` could salvage, and a blob split across runs.

#### Unpacking: a file the unpacker panics on goes to review

- Options: let the panic end the queue's run, as any panic in it does, or send the entry to the owner's review.
- Chosen: review, with the panic logged.
  The entry would otherwise fail every run of the queue, and every entry behind it would wait.
- Against: a bug in the unpacker then shows as a review item rather than as a crash someone notices.

### Link checks

#### GitHub's commit-history pages

- Options: a card flag such as "logs link not checked", which changes the card's format; a warning to the author only; or nothing.
- Chosen: a warning at submission and no flag, as for a link on an unlisted host; no row is stored, so no recheck.
  A redirect to such a page counts as live, as one to a download host does.
  The match is github.com with `commits` as the third path segment, decoded, so `pull/<n>/commits/<hash>` stays checked.
- Against: a reader of the card can't tell the `logs` link went unchecked; `entry-check` offline doesn't warn either, since the rule is about what the server fetches.

#### What a link check counts as dead

- Options: count only 404 and 410 as dead, or any 4xx but 408 and 429; follow a redirect to a listed host's download domain, or count it live unfetched.
- Chosen: any 4xx but 408 and 429 is dead, since Hugging Face answers 401 for a private or missing repository.
  A redirect to `codeload.github.com`, `raw.githubusercontent.com`, `objects.githubusercontent.com`, or a host under `huggingface.co` or `hf.co`, is live without a fetch, so the server never downloads a file.
  A link with a user or a port is dead unfetched.
- Against: a 403 from GitHub's web pages can mean a rate limit rather than a missing page, and such a link would be flagged dead until a recheck a week later.

#### When the link checks run, and for how long

- Options: in the submission request, or in a job during the hour's hold, with the warnings shown at `GET /e/<id>`.
- Chosen: in the request, after every other check, each URL once, 8 at a time, each fetch at most 10 s, all of them at most 20 s; what is unanswered then counts as "retry later".
  The warnings arrive in the reply, and a bad pin can be refused.
- Against: a submission with a slow host takes up to 20 s longer, and a batch of 100 entries with many distinct links may leave some unchecked until a recheck an hour later.

#### Which branches the fork check compares

- Options: the first 10 other branches in GitHub's list (alphabetical), or the 10 most recently pushed, which needs a call per branch to learn.
- Chosen: the first 10 in the list, skipping the one whose head is the default branch's.
  One call lists up to 100.
- Against: in a repository with many branches, the commit's branch may sort late, and an honest entry is flagged "commit on no branch"; the warning says how many branches the server checked.

#### GitHub's commit page as the fork check's first step

- Options: when the page's empty answer can't be confirmed, because the confirming compare can't run, leave the pin "not verified" or trust the page and flag "commit on no branch"; note a page the compare overrules in the log only, or also somewhere the owner looks, such as `/staging/status`.
- Chosen: "not verified", retried within the hour, since a check that couldn't run never says more; a log line, and a count on `/staging/status` since the server started.
  A page answer that lists a branch or tag is trusted without a call.
  The server falls back to the API on any answer but 200 with both lists of real entries.
  The page gets the same 10 s as any fetch, so one that hangs leaves the API path half of the request's 20 s.
- Against: a page that broke into answering empty lists costs one API call for each GitHub link, and shows up only as a growing count on `/staging/status` and warnings in `journalctl`.
  And only a commit on the default branch is overruled and logged: one on another branch, or only in a tag, would still be flagged "commit on no branch", with the fork warning and no log line, until the page answered right again.
  And the page's own limits aren't published, so a busy day could earn github.com a pause, which also pauses fetching github.com links.

#### A commit on no branch: the order of the calls, and when it is checked again

- Options: list the tags before or after comparing the commit with the other branches; check every tag, or the first 100; recheck a no-branch commit within the hour, as an unconfirmed pin is, or weekly.
- Chosen: tags right after the default branch, so a commit its author tagged costs 2 calls, not 12; the first 100 tags, in one call, matched by the commit each points at; weekly, since "on no branch" is an answer, not a failed check.
  An author who tags the commit can PUT a pending entry to have it checked at once.
- Against: a repository with over 100 tags may hold the one that matters further down, and a public entry whose author tags the commit keeps its flag for up to a week.

#### What a recheck covers, and when

- Options: recheck only the due links, or every link of an entry that has one; skip entries by status.
  Retry a link that couldn't be checked every hour, or an hour then doubling.
- Chosen: every link of an entry with a link whose next check has come, so a link that died since gets its flag too; every status except rejected, withdrawn and removed.
  A dead link or a bad pin waits a week; a "retry later" link or an unconfirmed pin waits an hour, then 2, 4 and so on, at most a week, as GitHub advises for backing off.
  The job runs every minute, up to 100 entries a run.
  While fewer than three quarters of GitHub's hourly calls are left, or GitHub's API asked the server to wait, it holds each entry whose check may call that API, leaving submissions the rest of the server's half, and rechecks the others (#21).
  Such an entry has a `code` link to a GitHub commit not yet confirmed on a branch or tag; it waits whole, its other links too, with its next check and retries as they were.
  Each run reads and analyzes every held entry again, a minute apart, to see whether it still must wait; that grows with the entries held, and is small at the sizes expected.
  The other choice in #21, not taken without the owner: let the held entry's check run and refuse only its API calls, so a commit GitHub's commit page can confirm alone is confirmed sooner.
  Against that one: a pin whose check needs the API would then turn from "commit on no branch" to "pin not verified" on a public card, and spend a retry on a call nobody made.
- Against: a public entry whose links were all live at submission is never rechecked, so a repository deleted later keeps a clean card.
  And a host that is down for a day is retried 4 times that day, at 1, 3, 7 and 15 hours.

### Skills and their install

#### Staging installs of search 7 and submit 9 upgrade to files that name production

- Options: accept it, since a rerun of the installer against staging repairs it; or keep placeholders beside the production names for one more version, which the old upgrade steps would fill; or have staging's `/skill.md` name no newer version until those installs are gone.
- Chosen: accept it.
  An installed copy upgrades through its own installed `references/upgrade.md`, whose `sed` fills `{{BASE}}`, and the new files have none, so a staging copy upgraded from 7 or 9 searches production and checks production's `/skill.md`, sending no staging key.
  On production the same steps leave the tag's bytes, which is right.
  A rerun of the installer against staging repairs such a copy without a refusal: its `SKILL.md` names production, so the local-change check compares it with the tag as an install from production holds it, finds it unchanged, and replaces it with files pointed at staging.
- Against: until someone reruns the installer, a test machine's searches reach production and count in its search log, which the per-host header files were meant to keep apart.

#### What the installer's local-change check compares against, and what it ignores

- Options: compare an installed folder with its version's tag as an install from this site would hold it, as the owner's ruling reads; or as an install from the site its own `SKILL.md` names; and refuse every extra file, or ignore some.
- Chosen: the site the folder's `SKILL.md` names, in its line `File Drawer (<address>) is an archive`, which every tag of both skills has; `{{BASE}}` there means a copy straight from an old tag, compared with the tag's bytes.
  So an unchanged install from another site (staging, then production; a dev server on another port) and a staging copy an old upgrade left naming production pass, and a rerun replaces them, unless the copy is newer than the version the site names (below).
  An edited address line makes `SKILL.md` differ, which refuses.
  `.DS_Store`, which macOS Finder leaves in any folder it shows, is ignored as an extra file.
  A link at `~/.agents/skills/<name>` refuses, since the install would replace the link with a folder.
- Against: a human who pointed a staging install at production by hand would see that edit replaced; and ignoring `.DS_Store` bends "a file the tag's list doesn't" hold.

#### A rerun of the installer refuses a skill newer than the site names

- Options: refuse the whole run when an installed skill is newer than the version the site names; or leave the newer skill, install the other and exit 0; or install the older version and say so.
  And for the same version: install it again, or stop as the upgrade script does.
- Chosen: refuse, installing neither skill, as the installer does for a folder with local changes; the same version installs again, since a rerun repairs a copy pointed at another site.
  The comparison is the upgrade script's: versions are numbers of at most 9 digits, and the installer fails before fetching when the site names anything else, or one skill twice.
  The local-change check runs first, so a newer folder with a human's edits is refused with its files named.
  The refusal says the installed skills work as they are, and that a human who wants the older version moves the folder away and reruns; it gives no flag, since the page that names the version could also tell an agent to pass one.
- Against: a machine ahead of the site for an honest reason gets exit 1 from a rerun until the site catches up: one that installed from staging and then runs production's installer, one whose human copied a newer tag into `~/.agents/skills/`, and every upgraded machine after production is rolled back to an older commit.
  The agent then reports a failed install, though nothing is wrong with the skills in place.
  Leaving the newer skill and exiting 0 would spare them that, at the cost of "both or neither".
  The rule also covers only `~/.agents/skills/<name>`: a copy at `~/.claude/skills/<name>` with nothing at the first path is still replaced unchecked, as the next entry says.

#### A copy at `~/.claude/skills/<name>` isn't checked

- Options: check it like the folder in `~/.agents/skills/`, or leave it, as the owner said.
- Chosen: leave it.
  Where the installer made a copy there, because `ln -s` failed, a rerun replaces the copy, and edits a human made in it are lost.
- Against: Claude Code reads that copy, so on such a machine it is the folder a human would edit.

#### The installer comes from the site, not from a git tag

- Options: the server serves `crates/server/install.sh` at `/install.sh`; or `/skill.md` names a tagged copy on GitHub, as #73 proposed, so the site supplies no code at all.
- Chosen: the site, as the build's brief asked; production serves the repo's bytes, and `X-Filedrawer-Commit` names the commit to compare with.
- Against: a compromised server could serve another script to an agent that doesn't read it first, where a tag can only be planted through the deploy key.

#### Where the downloaded installer stays

- Options: `~/.config/filedrawer/install.sh`, kept after the install; a temporary file; or the installer deletes itself.
- Chosen: `~/.config/filedrawer/install.sh`, a fixed path the human can read before the second command runs (the owner wanted the download and the run apart, so the script can be audited), and can read again later.
- Against: it leaves a second file in the home folder, which a careful human may not expect.

#### The installer's mode has no default

- Options: `auto` when no mode is given, or refuse.
- Chosen: refuse, before anything is fetched, so an agent that drops the argument doesn't choose `auto` for its human.
- Against: a person running the script by hand must type the mode.

#### Skills installed by the human's own means don't upgrade themselves

- Options: the upgrade script stops and tells the human, in a line, that a version is out, for a copy outside `~/.agents/skills/` and `~/.claude/skills/`; or it tries to put the new folder where the copy is.
- Chosen: tell the human; their own tooling put the copy there, and its swap assumes the installer's folders.
- Against: the agent repeats the line once per task until the human upgrades.

#### `/skill.md`'s download command off production

- Options: the same plain command everywhere; or, on staging and dev, a command that also sends the header file when it exists.
- Chosen: the header file off production, since staging's Caddy refuses a request without the key, and usability scenarios reach `/skill.md` through `?key=`, which the next command doesn't carry.
- Against: staging's page differs from production's in more than its address.

### Pages for people

#### The daily email's details

The owner settled the email's sections and the rules in design.md's Admin section (#44); an agent chose these, after its brief review.
- Feedback: at most 20 items in full, the newest; past that the oldest are only counted.
  Against: an account may store 50 items a day (#60), over the 20 shown, so a busy day still hides some feedback from the owner until the triage agent reads it (#18), and nothing else shows it.
- Host pauses and GitHub's half running out are listed but don't send the email alone, and screen failures send it only while the last call failed.
  Against: a host that keeps pausing the server, or a screen that fails often but recovers each time, never reaches the owner on a quiet day.
- A failed run is retried an hour later, for the admins it didn't reach; a retry sends them the period up to the retry, so the admins it reached first miss what came between the two.
  Against: retrying sooner gets the email out sooner, but floods the log and healthchecks while SMTP is down.
- Past days' caps are judged against today's cap, and the email says "of today's cap".
  Against: after the owner changes the cap, a day may be reported as having run out, or not, wrongly.
- With no admin to send to, production pings `/fail`; staging and dev ping success.
  Every admin gets the email.
  Against: before the owner's `filedrawer admin grant` on production, healthchecks reports a failure every day.
- The first run, or one after a wrong clock, reports from the previous UTC midnight.
  Against: a first run late in the day reports up to 48 hours.

## Architecture

#### The unknown-field report: what it counts and shows

- Options: count entries, chains or accounts; order by any of them; show a field's values or only its name.
- Chosen: one row a field name, as written, with its accounts and its entries, where each stored version counts as an entry, and the times the first and the last were submitted.
  The fields most accounts use come first, then the ones in most entries, then by name, at most 200.
  Reason: a field one account adds to every entry is that account's habit, and one many accounts add on their own is a candidate for the format.
  Only the name shows, cut to 80 characters and escaped: it is author text, and a value can be long or private.
  Entries of every status count, since an agent adds a field before it knows what screening will say; a deleted entry's fields go with it.
- Against: names that differ only in case or spelling (`seed`, `seeds`, `Seed`) are separate rows, so a field can look rarer than it is.
  Past 200 fields the page says so and shows no more, and has no way to see the rest.

#### The unknown-field report: how many fields an entry adds to it

- Options, for #80: cap the fields the report counts per entry in the analyzer, with a warning past the cap, or where the server stores them, saying nothing; refuse an entry past the cap, or accept it.
- Chosen: the server stores an entry's first 50 unknown field names, each cut to 100 characters, and the reply is as before, a warning for every unknown field.
  The entry's text keeps every field, so the cap changes nothing its author can see, and a misspelled field past the 50th still gets its warning.
- Against: a field past an entry's 50th never reaches the report, and two long names that share their first 100 characters count as one.

#### An unknown `view` on the home page

- Options: refuse it with a 400 page, as `/search` refuses an unknown filter value; or show Recent, as a bare `/` does.
- Chosen: a 400, but only to a browser; an agent's `/` gets its plain text whatever `view` says (#107).
  A misspelled tab link then shows up rather than quietly showing another tab.
- Against: a person who edits the address by hand gets an error page where Recent would do.

#### Most read: what its cards and links show

- Options: each card's count beside its stamp ("12 reads"), or no count; links "← Back" and "More →", or "Newer" and "Older" as search has them; a line under the tabs saying what counts, or none.
- Chosen: the count, since how often an entry is read is what the tab is about, in the call strip beside the stamp, the server's own words, not on the analyzer's card; "Back" and "More", since the pages go by readers, not by date, and Recent's link to `/search` says "More" too; and the line "Readers in the last 30 days, through the skills or in a browser." (#107).
- Against: a count invites gaming (#114), and "More" on Recent leads to a search page that says "Older".

#### The read counter: who counts as whom

- Options, for each: a browser with a session from another site; a browser opening `/e/<id>.txt`; an agent's fetch tool that copies a browser's headers; a day's close right at midnight.
- Chosen (#107, after its brief reviews):
  - The session cookie is `SameSite=Lax`, so it comes with a link from another site, such as an email's: an author or the admin who arrives that way is known, and their read isn't counted.
  - A browser opening the plain text counts as a page read, by `Sec-Fetch-Dest: document`, the same rule that gives browsers pages.
  - A fetch tool that copies a browser's headers counts as `page`, and one that sends neither those nor the skills' User-Agent isn't counted; `page` means "a browser-like client", not "a person".
  - A browser's prefetch or prerender (`Sec-Purpose`, `Purpose: prefetch`) doesn't count.
  - A day closes 10 minutes after midnight UTC, so a read that began just before still lands in its day.
  - A read of a superseded version goes to the latest public version when it is counted; if that version is later removed, its counts stay with it.
  - Browsers send reduced User-Agents, so people on the same browser behind one address mostly count as one; the address with the User-Agent separates harnesses and models more than browsers.
- Against: each of these miscounts some honest reads, which matters only for a curiosity tab.

### Storage

#### When the reference keys and tags are rebuilt

- Options: at startup when the analyzer's `INDEX_VERSION` differs from the stored one, or at every startup.
- Chosen: on a version change, since a rebuild reads every entry.
  The keys and the tags are rebuilt together, in one read of every entry, under the version row `entry_references`, which binaries since that table already lower after a rollback; so the tags a rolled-back binary stored are rebuilt too once a newer one starts.
- Against: if a deploy rolls back to a binary from before `entry_references`, the entries it stores get no keys, and the next binary, finding the version current, doesn't fill them, so `cites=` misses them until each gets a new version.
  A rebuild at every startup would cost a read of every entry, still small before launch.

#### The nightly copy before the first deploy

- Options: the timer skips while production has no database (`ConditionPathExists`), or it runs and pings `/fail` each night until the first deploy (#41).
- Chosen: it skips, so healthchecks.io raises no alarm before production has anything to copy.
- Against: a database path that drifts from the unit's condition skips every night silently; healthchecks.io notices only once the check has had a first ping, and the daily email's missing-copy line is the only other sign.

#### The nightly copy refuses a copy dated after today

- Options: fail the run, keeping every copy, when the newest copy is dated after today (UTC); or copy and prune as usual.
- Chosen: fail (#41's brief review).
  A clock that ran ahead leaves a copy that pruning by name keeps for good and the daily email takes for fresh, hiding every later failure.
- Against: after a clock that ran ahead is fixed, every night fails until someone removes the future-dated copy by hand.

#### The restore drill's firewall

- Options: create the drill server behind a Hetzner firewall whose one outbound rule denies everything else, or boot it in rescue mode and remove production's unit links before its first boot.
- Chosen: the firewall (#41's brief review): a few console clicks, and nothing on the restored disk changes before the drill reads it.
- Against: at boot the timer's catch-up run still makes a copy and prunes the oldest on the drill's disk, and Hetzner always lets DNS to its own resolvers through, which rescue mode wouldn't need to trust.
  Hetzner's firewall FAQ (https://docs.hetzner.com/cloud/firewalls/faq/, checked 2026-11-10) confirms that with one or more outbound rules, traffic no rule matches is dropped.

#### How an upload's files are synced

- Options: sync each file and its directories as it is written, which costs about three syncs a file; or write a set's new files, sync the store's whole filesystem once (`syncfs`), rename them into place and sync once more.
- Chosen: two syncs of the filesystem a set, made before the upload takes the database's write lock.
  The time an upload holds the store's lock, which deletions wait for, then doesn't grow with a sync for each file, and other writers never wait on a file sync.
  When the records don't commit, the upload removes the bytes it wrote.
- Against: `syncfs` also flushes whatever else the disk holds unwritten, staging's and production's alike, so one upload's latency depends on the rest of the machine; and it is Linux only.

#### Rolling back across the change-of-address migrations

- Options: `crates/server/migration-rollback.txt` marks 0039 and 0040 `breaks`, so `ops/deploy-production` refuses to roll production back past them (as built); or `safe`.
- Chosen: `breaks`, since each rebuilds a table to widen a CHECK, `outgoing_mail`'s on `kind` and `github_starts`'s on `purpose`, which CODING_STANDARDS.md lists as breaking.
- Against: what an older binary does wrong is small.
  It deletes a `change` or `change_notice` email unsent, or sends an admin's as an empty notice, so a change asked for just before the rollback never gets its link; its account deletion leaves the address a pending change holds, until a newer binary's tidy; and it treats a `change_email` GitHub start as expired.
  `safe` would keep the rollback, and 0038's line calls a like case safe.

### API

#### JSON on every route

- Options: JSON wherever Accept asks for it, or plain text only for `/cards.txt`.
- Chosen: `/cards.txt` stays plain text, since its name says so; every other route, errors included, answers JSON on request.
- Against: an agent that always sends `Accept: application/json` gets text from one route.

### Mail

#### Mail when SMTP fails

- Options: show the same page and log the error, as here, or tell the person the email didn't go.
- Chosen: the same page, so the signup and reset pages never reveal whether an account exists.
  The account page can resend the verification email, and since #103 the queue retries a failed send for a day.
- Against: a person whose email never arrives has no hint why.

#### The mail queue's retry and drop rules (#103)

- Options: retry every failure until the row expires, or tell apart a failure of the row's own from one of the server.
- Chosen: an unparseable address, or a permanent refusal of the recipient (550, 551, 553), drops that row and the run goes on.
  A failure of the server's, no answer, closing (421), no TLS (454) or a refused login (530, 534, 535, 538), stops the run and leaves the row due; no run starts until the server's wait passes (1 minute, doubling to an hour), so signups and resets can't make the server retry SMTP.
  Any other reply, such as a full mailbox (452) or a greylist (450), puts that row off by its own wait, doubling the same way, and the run goes on.
  The reviews found that retrying a refused recipient forever let signups with bad addresses hold up everyone's mail, that dropping on every permanent error would empty the queue on a credentials mistake, and that a row the server turns away for now mustn't stop the run.
- Against: a server that answers 550 for a reason that isn't the recipient (some use it for policy blocks) loses those emails at once, and one whose other codes mean the server is down gets a run of tries, one a row, that each answer at once.

#### How long the mail queue keeps a row

- Options: drop at the link's expiry, as the issue put it, or after a day for every kind.
- Chosen: a row is dropped unsent a day after it was queued, every kind alike, since a day-old signup or reset email is stale; the link it would carry is minted at the send and works 24 hours from then.
  A dropped row keeps its kind, account, times and failure class, with no text, for a week, for the daily email to count; deleting the account deletes its rows, dropped ones too.
- Against: a week of rows is longer than the count needs, and a dropped row deleted with its account goes uncounted.

#### One waiting email per account and kind

- Options: queue each request, or replace a waiting row of the same kind.
- Chosen: the owner chose one waiting verify and reset email per account (2026-11-18); the "someone tried to sign up" warning is kept to one as well, since strangers can trigger it, and attachment notices are not.
  A row replaced while it is being sent can make one extra email.
- Against: none known.

#### Error text the mail queue keeps

- Options: keep the error's text for the status page, or only a class.
- Chosen: only a class, `address`, `refused 550`, `smtp 451` or `connect`, on the row and on `/staging/status`, since an SMTP reply can repeat the recipient's address.
- Against: the class alone may not say enough to find a fault, and no log keeps the error's text either.

#### Queued mail in the restore drill

- Options: start the sender in every `serve`, or skip it without SMTP settings.
- Chosen: every `serve`, so dev's log mailer still shows each email.
  The restore drill runs `serve` on production's copy with no SMTP, so it logs the waiting rows' emails with fresh links that work only against that copy, on a firewalled server deleted afterwards.
- Against: those addresses sit in the drill server's journal until it is deleted.

#### Reset's timing

- Options: keep reset as it was, queueing (a write) only for an existing account, or make both arms write.
- Chosen: as it was, since #103 asked for the timing to stay as even as it was; signup writes in both arms.
- Against: reset still takes a write longer when the address has an account.

### Calls to other services

#### An account's share of GitHub's API calls

- Options: count each account's calls per UTC hour, or in a token bucket that refills over an hour; in memory, or in SQLite as the hourly rate limits are.
- Chosen: a token bucket of a tenth of the server's half of the hour's calls (250 with a token), in memory beside the per-minute rate limits.
  It has no boundary at which an account gets two hours' worth at once, and a restart forgets at most one hour's count.
- Against: a deploy hands every account a full share again.

#### An account's link checks an hour

- Options: no limit of its own, leaving the daily allowance and the 60 writes a minute to bound the fetches; a per-account count of link checks an hour, refusing a request past it with a 429 or accepting it with its links unchecked until a recheck; or a server-wide rate per host.
- Chosen: each account's requests may cause 1,200 link checks an hour, a token bucket in memory beside its share of GitHub's API calls.
  A request's checks are each distinct URL it has the server fetch, plus each `code` link whose pin check asks GitHub or Zenodo; redirect hops aren't counted, and the server's own rechecks take none.
  They are taken together before the first fetch, and a request that needs more than the account has left is refused whole, with a 429 and `Retry-After`, as every limit answers.
  1,200 is what the largest request needs, 100 entries with 10 `logs` links and a `code` link each, so a full hour's checks never refuse one request.
  Reason: the allowance is read before the fetches, but a submission can still be refused after them, for a Zenodo concept record, and such a request takes no allowance, so without a count a refused request's fetches would cost its account nothing.
  Refusing, rather than accepting with the links unchecked, keeps a Zenodo concept record from being accepted because the account spent its checks first.
- Against: an agent submitting several large batches with many links in one hour is told to wait, up to an hour.
  A full bucket still lets one request send a host more than it allows a client that isn't logged in (Hugging Face about 100 pages in 5 minutes, Zenodo 60 a minute), so the host may ask the server to wait after a large batch, as before; a server-wide rate per host isn't built.
  The count is in memory, so a deploy hands every account a full hour again.

#### Pauses for hosts that ask the server to wait

- Options: keep the pauses in memory or in SQLite; treat every GitHub 403 as a rate limit, or only one that says so; cap the waits or not.
- Chosen: in memory, per host, with `api.github.com` for the API.
  A GitHub 403 counts as a rate limit only with no calls left, with `Retry-After`, or with "rate limit" in its message, since a 403 can also mean an account or repository is blocked.
  A page host's 503 pauses it only with `Retry-After`.
  `Retry-After` is held to a day, and a doubling wait to an hour.
  A pause only grows, and a normal answer ends the doubling but not the pause, since it may answer a fetch sent before the host asked.
  The recheck job holds the entries that may call GitHub's API while it is paused, so a long wait costs them no retries; a link on another host is rechecked meanwhile, and counts its own.
- Against: a restart forgets a pause, so a deploy during GitHub's secondary limit sends one call too soon; and a rate-limit 403 worded otherwise is read as a plain error, retried at the next submission.

### The server's own bounds

#### How many passwords are hashed at once

- Options: no bound, as before; a fixed number; or a number from the machine's cores.
  A request that finds no place free can wait without limit, wait a while and then be refused, or be refused at once.
- Chosen: a hash for every core the process may use but one, at least 1 and at most 4, so 1 on the 2-core VPS, where staging and production share the cores with Caddy.
  Each argon2id hash fills 19 MiB and takes a core for tens of milliseconds, so one at a time is about 20 logins a second, and 4 at once is 76 MiB.
  A request waits up to 5 s for its place, then gets a 429 with `Retry-After: 5`, the shape every limit answers with.
  Login, signup, password reset and account deletion share the places, and a login for an address with no account takes one like any other.
  A request takes its place before it counts against any hourly or daily limit, so a refusal for want of a place costs it nothing.
- Against: 20 logins a second is far more than people need, but the queue is shared and first come, first served, so a busy period refuses honest logins too.
  A place per client in the queue would stop that, at the cost of more state.
  A request holds its place while it reads its limits and its account from the database, not only while it hashes, so a slow write delays every login behind it.

#### Requests in progress per client

- Options: only the per-minute rate, as before; or also a bound on how many requests one client IP has in progress at once.
- Chosen: 32 in progress per client IP (IPv6 by /64), counted from a request's headers until the last of its reply is written; past that, a 429 with `Retry-After: 1`.
  A refused request takes nothing from the per-minute rate.
  Reason: the rate counts a request when it arrives, however long it then stays open, as a slow upload or download does; with this, one client holds at most 32 of the 512 connections.
  A browser opens about 6 connections to a host, and an agent sends its requests one after another.
- Against: a room behind one address, such as a sprint, shares the 32.
  Requests take milliseconds, so 32 at once is hundreds a second, but 32 people uploading attachments on a slow link at the same moment would see a 429 and retry.
  Unlike signups, an invite code doesn't lift it.

#### How long a request's headers and body may take

- Options: no limits in the server, relying on Caddy, as before; or the server's own.
  For the body: one fixed time, a time that grows with the declared length, or a lowest rate.
- Chosen: the server's own, whatever stands in front of it.
  Headers: 150 s from when the connection opens or from the reply before, which is also how long a connection may sit idle.
  It is longer than the 2 minutes Caddy keeps an idle connection to the server for reuse, so Caddy closes an idle connection first; were the server first, a POST that Caddy sent into the closing connection would fail, since Caddy doesn't send a POST's body twice.
  Caddy's 2 minutes is its default, which `ops/remote/caddy.sh` doesn't pin.
  Body: 60 s for each MiB the `Content-Length` starts, at least 1 and at most 4 (the largest body any route reads), and 60 s with no declared length; past it, a 408 that says so, and the connection closes.
  A link of 140 kbit/s keeps up with that.
  The time runs from the headers and bytes that keep arriving buy no more; a handler that has its whole body takes as long as it needs, as a series' screening does.
  Each route refuses a declared length past the largest body it reads at once, with a 413, before reading any of it: 4 MiB for `POST /attachments`, 1 MiB for every other route, and 64 KiB for the forms of signup, verification, login and password reset, whose longest, a signup, is at most about 13 KiB.
  So a body the server waits for is one its route takes, and only an attachment set can be given more than 60 s.
- Against: 150 s is long for a header.
  It rests on the server's port being open to the same machine only, where Caddy sends a request on once its headers are complete; a server reachable directly would want about 30 s and Caddy's reuse time set below it.
  A chunked upload with no declared length gets 60 s however large it is.
  A form gets the 60 s of a MiB, though it is a few KiB; a time counted by the byte, with a floor of some seconds, would hold a stalled form for less, and change the time every other body gets.

#### How many connections are open at once

- Options: no bound, as before, up to the 1,024 file descriptors systemd gives the service; a bound in the server; or a higher `LimitNOFILE` in the unit, or both.
- Chosen: 512 in the server; further connections wait, unaccepted, until one closes.
  The other half of the descriptors stays free for the database's 8 connections, attached files, which are read on up to 512 blocking threads, outbound calls and the log.
  The unit's limit stays as it is.
- Against: the deploy's health check shares the bound.
  Raising `LimitNOFILE` and the bound together would give more room, at the cost of a provisioning rerun.

### Staging and checks

#### Usability runs' public leftovers

- Options: delete each run's account at its end and list any entry already public for the owner to take down with `ops/remove-entries`; or give scripts an admin credential so the run removes them itself.
- Chosen: the first (the owner, 2026-11-10, for deleting; the list and the script are the agent's).
  Admin pages answer only the owner's browser session by design, and most runs post near their end, inside the hour's hold, which the account's deletion removes.
- Against: a run that posts early leaves a public entry on staging until the owner runs the script, and a rerun's search may find it meanwhile.

#### Codex's login in a usability run

- Options: give the run's Codex a copy of the driver's `auth.json` without its refresh token, as drive-agent does; or with it, writing a refreshed copy back.
- Chosen: without it, so the agent under test never holds a durable credential and nothing it writes reaches the driver's login.
- Against: Codex can't refresh, so a run longer than its access token's life fails partway; refresh by using Codex before a long round.

#### Usability accounts and the review guard

- Options: sign up each usability account from the sandbox's IP, as now; or exempt usability accounts once #17 spreads the review guard by signup /64.
- Chosen: as now, since #17 isn't built.
  The invite code already spares the per-IP signup limit.
- Against: once #17 lands, a held usability entry would send every account from the sandbox's /64, the owner's own staging account included, to review.

#### The smoke-test account's screening note

- Options: say "not screened" in the entry's screening note, or show it as a separate line the server owns.
  (The owner decided on 2026-11-06 that the smoke account isn't screened at all, so the smoke test calls no model.)
- Chosen: the screening note, as `staging smoke account: not screened`.
  It is staging only (production refuses `FILEDRAWER_SMOKE_ACCOUNT`), and it needs no new card line.
- Against: entry-format.md defines the screening note as the screen's doubt about `outcome`, quoted as model text, so the server's own words read as the model's.

#### `FILEDRAWER_LINK_CHECKS=off`

- Options: a setting that turns the checks off, or no setting, with every fixture the binary's tests submit kept free of listed-host links.
- Chosen: the setting, which production refuses, like `FILEDRAWER_SCREENER=pass-all`.
  `crates/server/tests/it/smoke.rs` sets it, since `ops/smoke-staging eval` submits the `ops/screen-eval/` cases, which link github.com.
- Against: one more setting, and the binary's tests no longer exercise the checks; the router tests do, with a fake web.

#### Staging without a GitHub token

- Options: staging refuses to start without `FILEDRAWER_GITHUB_TOKEN`, as production does, or starts and logs a warning.
- Chosen: it starts, with the warning, so a deploy before `ops/provision-vps github` has run doesn't fail.
- Against: staging can run for days at 60 calls an hour, and when GitHub's commit page doesn't answer, most GitHub links get "pin not verified", which only `/staging/status` ("github allowance: 60 calls an hour") and the log say.

#### agent-pre-commit without `CHECK_BASE`

- Options: drop the hook's own record of passed trees (`checks-passed` in the git dir) and run every check whenever `CHECK_BASE` is unset, or keep the record as a fallback for when llm-toolbox's `agent-commit-gate` isn't on PATH.
- Chosen: drop it.
  The gate already records each pass, and a base it offers is by construction a tree that passed this same check, which is what the record guarded; one record can't drift from another.
- Against: an agent committing on the owner's host, which has no `agent-commit-gate`, now runs every check, as CI does, where since 68c462b it ran only what its staged paths needed.

#### The daily mutants run's shards

- Options: a shard per 40 mutants, at most 4 (`bin/mutants-shards`); more shards, up to the full run's 16 at once; or one shard.
- Chosen: 40 a shard, at most 4.
  A day's changes make a few mutants, one shard's worth, and 4 shards beside a full run's 16 stay within the org's 20 concurrent jobs.
- Against: the first daily run, with no covered run before it, tests the lines changed since the workflow was added, close to a full run: about 2300 mutants on 4 shards, some 2.5 hours, under the shard job's 330 minutes but longer than a full run takes.
  A full run started by hand on main first makes that daily run small.

#### Dependabot

An agent added this on 2026-11-20, with the owner away; deleting `.github/dependabot.yml` undoes it.

- Options: pin each workflow action to a commit and move the pins by hand; pin them and let Dependabot move them, with or without the crates in `Cargo.lock`; or leave actions on tags.
- Chosen: pinned, with Dependabot for `github-actions` and `cargo`, weekly, grouped into one pull request for each, and only releases at least 7 days old (`cooldown`).
  A pin nothing moves goes stale, and a tag is whatever was last pushed to it.
- Against: up to two pull requests a week that the owner merges on GitHub, outside wt-ship, each deploying staging; the cargo one can widen a requirement in a `Cargo.toml`, a major version included.
  Dependabot's security updates and alerts are repository settings, which this file doesn't turn on.

#### What a build job may restore

An agent chose this on 2026-11-20, with the owner away.

- Options: let `build-staging` restore `Swatinem/rust-cache` and the release's `wheel` and `sdist` jobs uv's cache, as before; or build what ships from the checkout alone.
- Chosen: no cache in those three jobs.
  Any job on main can save a cache entry the next run restores, so a restored one makes every such job's tools part of what built the server's binary or a published package.
- Against: each push to main builds the server cold, about 2 minutes where a warm build took 40 seconds, so staging deploys that much later.

#### The daily advisories check

An agent added this on 2026-11-20, with the owner away; deleting `.github/workflows/advisories.yml` undoes it.

- Options: report every RustSec finding for `Cargo.lock`, or only vulnerabilities; one issue per advisory, or one issue for all; after the owner closes the issue by hand, open a new one each day the advisories still apply, reopen it, or take the close as accepting what it lists.
- Chosen: vulnerabilities only, in one issue the workflow rewrites and closes, and a close by hand accepts the advisories listed, so a new issue opens only for one not listed.
  cargo-audit's warnings (unmaintained, unsound, yanked) name crates nobody here can patch, and would keep an issue open for good; `.cargo/audit.toml` is where an accepted advisory is written down.
- Against: an unsound or unmaintained dependency is never reported; an advisory accepted by closing the issue leaves no trace in the repo, and stays accepted if it goes away and comes back.
  A run that fails (RustSec or GitHub down, a cargo-audit too old to read the database) shows only as a red run of a scheduled workflow, which GitHub mails to whoever last changed its schedule; and GitHub stops scheduled workflows in a repo with no activity for 60 days.

### Production deploys

#### A deploy after the tip's CI failed (#118)

An agent chose this on 2026-11-22, building #118.

- Options: with no COMMIT, when the tip's CI already ended without deploy-staging and smoke-staging passing, promote the older commit staging still runs, as before #118; or stop, naming the failed jobs and staging's commit.
- Chosen: stop, unless the only failures are other jobs, such as screen-staging, since then staging runs the tip and promoting it is what was meant; a run the script waited for stops on any failure, and a rerun then promotes it when only another job failed.
- Against: promoting staging's older commit while the tip is broken now takes naming it.
  A tip with no run at all (`[skip ci]`) waits 60 s for one before going on as before.

#### Who may open the apps' ports

- Options: reject only the users found to reach them (as before, staging's user to 8080); allow only Caddy and root; or per port, each app's own Caddy site alone.
- Chosen: only Caddy and root, for new TCP connections to 8080, 8081 and 2019 at any local address (`fib daddr type local`), so a listener left on the public address is covered too; a packet of an open connection or of a socket in TIME_WAIT passes, since it has no user to match.
  `loopback.sh` runs from staging's Caddy stage as well as production's, refuses before Caddy is installed, and after loading checks that Caddy still connects to each listening app and `nobody` can't, putting the previous rules back when Caddy can't.
  The audit reports Caddy's reach and whether the unit is active, not only enabled.
- Against: an owner's `ssh -L` tunnel to an app's port works only as root now.
  A load that fails at boot keeps production down, since its unit requires the rules, and leaves staging's port open, since staging's doesn't; the audit shows it, nothing alerts.

#### Production's commit is public

- Options: `X-Filedrawer-Commit` on `/skill.md` for anyone, as #42 asked; or behind a key, with `ops/deploy-production` reading it over ssh instead.
- Chosen: public; the repo is public anyway.
- Against: it shows when production lags behind a public security fix.

#### CI's smoke test doesn't check which commit it ran against

- Options: pass `--commit` to CI's smoke run; or leave it out.
- Chosen: leave it out.
  `deploy-staging` and `smoke-staging` are in separate concurrency groups, so after two close pushes the older run's smoke can run against the newer binary, and `--commit` would fail it as a false alarm.
- Against: such a smoke run certifies a commit it didn't test, and `ops/deploy-production` takes that run's pass for the older commit; one concurrency group for both jobs would fix it.

#### A rollback the migration lint can't see

- Options: lint the migrations only; or also make the code tolerate enum values it doesn't know.
- Chosen: the lint, with CODING_STANDARDS.md asking a change that writes a new value into an unchecked TEXT column to add a `SELECT 1;` migration marked `breaks`.
- Against: it depends on the author noticing; an older binary that can't decode a row fails on it, for as long as the rollback lasts.
  Rows an older binary writes also keep a new column's default after the next deploy, which the standard asks to be right for them.

#### Re-running an old CI run moves staging back

- Options: say so in `ops/deploy-production`; or check that staging's commit is `origin/main`'s newest that passed.
- Chosen: neither: the script no longer advises re-runs, and asks before any commit but staging's.
- Against: a re-run now succeeds on staging even across a migration, and the script's default then promotes that older commit without asking.

#### `ops/deploy-production` updates the server's install and backup scripts

- Options: replace `filedrawer-install` and `filedrawer-backup` on the server when they differ from `origin/main`'s; or refuse and ask the owner to rerun `ops/provision-vps production`.
- Chosen: replace them, saying so, from `origin/main`'s text (the checkout must be at `origin/main`).
  The install's lock and copy and the nightly copy's matching lock must be on the server for the deploy to keep its promises.
- Against: the deploy then changes more than the binary, through a path the wizard also owns.

#### What the Caddy guard's check covers (#100)

- Options: `caddy validate` alone; or that plus requests through Caddy to each site (chosen); or those plus the files hosts and a field-test key.
- Chosen: validate, production's `/tags` and www's redirect, staging's `robots.txt` and its key gate both ways, since the breaking changes Caddy lists mostly refuse configs or change what requests get through.
- Against: the files hosts and an open field-test key go unchecked, so a release that breaks only those stays installed.

#### Clearing a rollback the guard made (#100)

- Options: a config fix both versions take, then unhold and upgrade (chosen); or a mode of the guard that unholds, upgrades and rewrites the config in one checked step.
- Chosen: the first, since a breaking change mostly refuses a config that never did what it looked like, so the fix usually suits both.
- Against: a fix only the new version takes can't land while the old one is held, which `caddy.sh`'s validate refuses, and needs hands on the server.

#### `--force-confold` for every package on the server (#100)

- Options: set `Dpkg::Options` to keep an edited conffile, server-wide (chosen); or leave dpkg's default.
- Chosen: server-wide, since unattended-upgrades skips a package whose conffile would prompt, and caddy.sh rewrites Caddy's packaged Caddyfile, so a Caddy release that changed it would never install.
- Against: every package now keeps its edited conffile without a word, where before unattended-upgrades left such an upgrade for a person.

#### The outbound probe's order (#102)

- Options: the new binary twice, then the previous one (the order the owner approved); or the new one, the previous one, then the new one again (chosen).
- Chosen: the second, from the brief's review: a host that comes back while the previous binary is probed would otherwise read as a regression and roll back a good binary.
- Against: one more probe for each deploy that fails it, so up to three SMTP logins while mail may already be failing.

#### No automatic rollback after a migration (#102)

- Options: roll the binary back whenever the previous one gets out (keeping every write); or keep the new one when it changed the schema, or when the schema can't be read (chosen); or restore production's deploy copy too.
- Chosen: keep it, since design.md refuses a rollback across a migration that breaks it and the server holds no copy of `crates/server/migration-rollback.txt`; `ops/deploy-production` with the commit before checks it.
  Restoring the copy would drop the writes the new binary took while it served.
- Against: most migrations don't break a rollback, so a binary at fault keeps serving until the owner acts, and staging's only way back is a revert.

#### What the probe leaves out or passes on (#102)

- Options: copy each `[Service]` line of the installed unit and ignore drop-ins; or refuse to probe a unit with a drop-in (chosen); and pass on the binary's output as it is, or only its own line shapes (chosen).
- Chosen: no drop-ins exist, and a probe that misses one could pass a unit that fails; the binary is the deploy key holder's code, so only `reached`, `skipped` and `can't reach` lines of printable ASCII, and a few such lines of its stderr, reach CI's log.
- Against: a drop-in added by hand makes every deploy exit 3 until it is folded into service.sh, and a binary's own diagnostics beyond those lines are only in the journal.

#### Where Caddy's admin API listens

An agent chose these on 2026-11-20, with the owner away.

- Options: turn the admin API off (`admin off`), with a systemd drop-in that makes a reload a restart; a unix socket in `/run/caddy`, with a drop-in that makes the folder (`RuntimeDirectory=`); or a unix socket in Caddy's home, `/var/lib/caddy/admin.sock`, which the package already makes.
- Chosen: the socket in Caddy's home, mode 0200, so `caddy` and root alone open it.
  The packaged unit's reload runs as `caddy` and takes the address from the Caddyfile, so it works unchanged, and with no drop-in a package upgrade has nothing to lose and Caddy nothing it can't start without.
  The first run after the change restarts Caddy, a few seconds without the sites, since the running Caddy doesn't listen on the socket yet; later runs reload.
- Against: with `admin off` there would be no admin API at all, where now a process running as `caddy` or root still has one; but each field-test key change would then restart Caddy and drop its connections.
- The nftables table (`ops/remote/loopback.sh`) could also reject loopback's port 2019 for every user but `caddy` and root, a second layer should a hand-written Caddyfile ever bring the default back; not built, since `ops/audit-vps` already shows who reaches the port.

#### Caddy's limit on a request body

- Options: Caddy's defaults, which end a request whose headers take over a minute or whose body stops arriving for a minute, but put no limit on a body that keeps arriving; a limit on the whole body (`read_body`); or a lowest rate (`read_body_idle` with a rate).
- Chosen: `read_body 5m` for the whole server, both environments, in the Caddyfile's global options, since Caddy takes it per server and not per site.
  It sits above the server's own longest limit for a body, 4 minutes, so a client gets the server's 408 and its reason; Caddy's limit holds if the server's ever doesn't.
- Against: one run of either environment's Caddy stage changes the limit for both.
  A lowest rate would end a slow body sooner than 5 minutes.

### Reboots

#### Where the reboot check lives

- Options: a script of its own, `filedrawer-reboot-check`, with its own units and a timer at 05:00 UTC, and the guard's mail, ping and site check moved into `filedrawer-alerts.sh`, which both source (chosen); or a mode of `caddy-upgrade-check`; or unattended-upgrades' `Automatic-Reboot`.
- Chosen: its own script, since the guard's lock, state and 08:00 timer are about Caddy, and `Automatic-Reboot` has no wait of days, no notice and no deploy check.
  Its stage is `ops/provision-vps reboots`, run by production's wizard after the guard's, and both stages install the library and both scripts.
  A dpkg hook starts the check after each dpkg run, so a kernel installed at 06:30 is mailed within minutes rather than at the next 05:00, which gives a day more notice for the same reboot.
- Against: a library on the server is one more file two stages must keep in step, and the guard's healthchecks.io check now hears from two jobs.

#### When the reboot check reboots

- Options for the clock: from the first sighting, as the owner put it, or from the notice's mail (chosen); they differ only when that mail fails.
- Chosen: from the mail, and no reboot until a notice is mailed, so every reboot comes with 4 days' notice; with mail failing, each 05:00 run pings `/fail` on the guard's check, whose alerts go to an address outside filedrawer.dev.
- Options for what puts it off: nothing; locks only; locks, apt and logins (chosen).
- Chosen: the staging deploy's lock, production's backups lock (its deploys and the nightly copy), the guard's lock or a rollback it left waiting, dpkg's lock, apt's daily units, any logind session of class `user` not `closing`, or logind not answering; it reads `/proc/locks` and takes no lock until the reboot, tries each minute until 05:25, then puts the day off and mails why.
  A session left open every day puts it off every day, which the daily mail shows.
- Options against a loop: rely on `/run` being cleared at boot; or also check the kernel that booted (chosen).
- Chosen: after its reboot it mails the running kernel and flags one older than the newest it rebooted for, and a `reboot-required` back at boot is recorded in `stuck`, so no second reboot starts until a new package asks.
- Not done: a check that the clock is synchronized (a server without an NTP daemon would never reboot), and a healthchecks.io check of its own (a new ping URL is a new secret, which the owner ruled out); the reboot check pings only `/fail`, since a success would hide a failing guard, so a failure it keeps finding shows as down at 05:00 and up at the guard's 08:00 success.
- Against: the gaps between `ops/deploy-production`'s ssh calls hold no lock, so a production deploy started near 05:00 on the reboot's day can be cut off; the notice says not to deploy then.

### Memory check

#### Which healthchecks.io check the memory timer pings

- Options: a check of its own, with a new ping URL in `/etc/filedrawer/memory.env` (chosen); or the Caddy guard's check, pinging only `/fail`, as the reboot check does, with no new secret.
- Chosen: its own check, asked for by its own stage, `ops/provision-vps memory`, which production's run also runs (#64).
  The issue asked for a check beside the others, and a check named for memory says what is wrong without opening its log; it pings every 5 minutes, so it also shows when the timer stops.
- Against: the owner ruled out a new secret for the reboot check (2026-11-21), and this adds one; on the guard's check a lasting memory problem would show as down after each run and up at the guard's 08:00 success, still once a day.

#### What counts as swap in use

- Options: swap activity, pages swapped out since the last run (`/proc/vmstat`'s `pswpout`, its last value kept in the unit's `StateDirectory`) (chosen, pending the owner's ruling on #64); any swap held, SwapTotal less SwapFree above 0, as design.md words it; or `vm.swappiness=10` in base.sh plus a threshold on swap held.
- Chosen: activity.
  At Debian's default swappiness of 60 the kernel swaps out idle pages while the page cache fills the RAM, as it does by design here, and a swapped-out page stays counted until it is read back or swap is turned off, so "any swap held" would stay failed on a healthy server, and healthchecks.io would then say nothing of a real RAM alert.
  The first run, and the first after a reboot resets the counter, only record it.
- Against: a single page swapped out in a burst still fails one run, and recovers at the next; a server that swapped heavily once and stopped shows nothing after one run, where "held" would keep showing it.

### Packages

#### The analyzer's library name

- Options: keep `[lib] name = "analyzer"` as published, or use the package's own name, `filedrawer_analyzer`.
- Chosen: `filedrawer_analyzer`, with the server's Cargo.toml renaming the dependency `analyzer`, so no server code changed.
  A crate on crates.io whose library is called `analyzer` reads as a generic name to anyone who depends on it, and docs.rs shows it under that name.
- Against: the server and the analyzer's own tests now call the same library by two names.

#### Versions

- Options: start both crates and the PyPI package at 0.1.0, or at 0.0.x; keep the two crates' versions apart, or in step.
- Chosen: 0.1.0 each, versioned apart; the PyPI and npm packages take `crates/cli`'s version.
  The CLI's version moves with what agents run, and the analyzer's with its API.
- Against: a version in step would let one bump cover both, and 0.0.x would say more plainly that nothing is stable yet.

#### Wheel tags and the release's guards

- Options: tag the static Linux wheels manylinux only, musllinux only, or both; let any branch's run publish, or only main's.
- Chosen: both tags on each static Linux wheel, as uv's own wheels do, so pip and uv on glibc and on musl systems take the same file.
  `ops/claim-names` makes the `pypi` and `npm` environments open to main only, and the workflow refuses files whose version isn't the one typed in.
- Against: a release from a branch would let a fix go out before it lands.

#### Finishing a partial PyPI upload

An agent chose these with #87's fix, after its brief review.

- Options: when PyPI has some of a version's files, upload the rest from whichever commit the rerun builds; or first compare PyPI's sdist with the rerun's and refuse a difference.
- Chosen: upload the rest.
  A release already fills in a registry that lacks the version from main's tip, so a later run at a commit with no version bump can finish what an earlier commit's run began; the sdist goes first, in an upload of its own, so it is the file least likely to be the one missing.
- Against: one version's wheels can then come from two commits, and nothing says so.
- A version PyPI can never complete, such as one with a file deleted there, fails in the publish job on every run until the version is bumped; before, it passed with the files missing.
- A file PyPI has that the run didn't build is a warning, since no run can remove it.
  The plan counts it among the version's files, so with one such file and one missing, the plan calls the version whole and nothing uploads the missing one.

#### What a release checks

- Options: release once CI's `check` job passes, or once the whole CI run does; compare the analyzer with crates.io's crate file by file, or by git tags of each release; pin the CLI's requirement on the analyzer with `=`, or with a caret.
- Chosen: once `check` passes and the commit is still main's tip, so staging's deploy and smoke runs, which can fail for reasons of their own, don't hold a release back; file by file against crates.io (`Cargo.toml.orig` in, the Cargo.toml cargo writes out), since CI can't push tags without a deploy key; `=`, so `cargo install filedrawer` builds with the analyzer the binaries hold, `--locked` or not (`bin/release-plan`; #84).
- Against: a release can go out from a commit staging never ran; a workspace-level change (`edition`, `license`) shows only in the Cargo.toml cargo writes, so it slips past the comparison; and `=` makes every analyzer bump a CLI edit too.

#### How ops/release-cli resumes

An agent chose these on 2026-11-21, building the owner's one-command release, after a review of its brief.

- Options: keep a state file of the step reached; or work the step out on each run from git, GitHub and the registries.
- Chosen: from what is true.
  Every registry has the checkout's version and nothing in `crates/cli`, `crates/analyzer`, `Cargo.lock` or the workspace's `Cargo.toml` changed since the commit that set it: nothing to do.
  Every registry has it and those changed: a new release, bumped.
  A registry lacks it: no bump; push if ahead, wait for `check`, then watch a release run in progress for that commit or start one, even after one that succeeded, since the workflow publishes only what's missing.
  It refuses to finish a version crates.io has and another registry lacks when those files changed since the commit that set it, since that would publish one version from two commits; while crates.io lacks it nothing of it is out, so a fix since, say for a failed `check`, goes into it.
  A "no" at the push drops the commit that run made, so the next run starts clean; a commit an earlier run made stays.
  Before the commit it runs `cargo metadata --locked` and `bin/release-plan plan` on the host, so the refusals CI would give come before the push.
  It waits about an hour for `check` and five minutes for the registries, then stops saying to rerun; it refuses with `AI_AGENT` set or while the main tree's `CLAUDE_WORKING` flag is there.
  It is Python in `ops/`, as `ops/remove-entries` is, with a `uv run --script` shebang so it gets Python 3.11's `tomllib` on an older Ubuntu.
- Against: a `Cargo.lock` or workspace `Cargo.toml` change for the server alone counts as a change to what ships, and makes a rerun bump an identical CLI; a registry whose cache lags past five minutes makes a rerun start the run again, which then fails on npm's refusal to publish a version twice, though nothing is lost.
- The analyzer takes the CLI's level, as the owner's brief put it, though "Versions" above says the analyzer's version moves with its API: a patch release of the CLI with a breaking analyzer change gets a patch analyzer, and `major` makes the analyzer's next version a major one with no API reason.
  Options: the same level, as built; or the analyzer by patch unless told otherwise, with a flag for its level.

#### npm's trusted publishers, set up again on each run

- Options: for a package npm has only as the 0.0.0 placeholder, set up its trusted publisher afresh on each run of `ops/claim-names` (revoke any, then add); add one only where `npm trust list` shows none; or ask the owner to look at the package's settings page.
- Chosen: afresh, right before the release stage, since `npm trust list` doesn't say whether a publisher has lapsed after its 2 days (npm/cli v11.21.2, `lib/trust-cmd.js`), so a rerun days after a failed release would otherwise keep one that no longer works (#84).
  A released package's publisher has been used, so it is left alone.
- Against: each rerun before the first release asks for up to three second factors a package, eighteen in all, where most reruns come within the 2 days and need none.

#### Where the npm launcher looks for its binary

An agent chose this on 2026-11-20, with the owner away.

- Options: find the platform package with node's `require.resolve`, as 0.1.0 does; or look by path only where the launcher's own install puts it.
- Chosen: by path, in the launcher's own `node_modules` and in the `@filedrawer` folder beside it, which covers npm (local, global and `npx`), pnpm, Bun and Yarn's `node_modules` linker.
  `require.resolve` goes on to the `node_modules` of every folder above and to `$NODE_PATH` when npm installed no platform package, and would run a same-named package it found there.
- Against: Yarn's Plug'n'Play keeps no `node_modules`, so the launcher finds nothing there and says no binary was installed; and a launcher installed under an alias (`npm install fd@npm:@filedrawer/cli`) no longer finds the package npm put under `@filedrawer`.
  It takes effect with the next CLI release.

#### Where login lands

- Options: always the account page; the page a GET with no session asked for, carried as `/login?next=`, or in a cookie.
- Chosen: `next`, taken only when it names a page on this site: one leading `/`, visible ASCII, no `\`, which browsers read as `/`; anything else lands on the account page.
  A POST with no session goes to the plain login page, since a form's target is no page to land on.
- Against: an admin whose session ended before pressing a form's button lands on the account page, not back on the form; and a `\` in a query is refused, though it couldn't lead off the site.

#### What a debug entry-check counts as its sources

An agent chose this on 2026-11-23, with the owner's decision that a stale debug build refuses (design.md, Distribution).

- Options: hash the `.rs` files under `crates/analyzer/src` and `crates/cli/src`, their two `Cargo.toml` files, `crates/cli/build.rs` and `Cargo.lock`; or the same without `Cargo.lock`; or also the root `Cargo.toml` and `.cargo/config.toml`, which set profiles and flags.
- Chosen: the first.
  A dependency moved in `Cargo.lock` changes what the binary does as surely as a source edit.
- Against: a `Cargo.lock` change for the server alone, as a Dependabot bump, makes a debug entry-check refuse until it is rebuilt, though the rebuild changes nothing it does; and the root `Cargo.toml` and `.cargo/config.toml` can change a build without a refusal.
