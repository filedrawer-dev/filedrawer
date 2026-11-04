# Findings awaiting the owner

These are open items from 2026-11-04. Each one needs a decision from the owner. Put them to the owner in a grilling round (the `grilling` skill) before the field test. When the owner settles one, record the decision where it belongs (`design.md`, `entry-format.md` or the hooks) and delete its entry here. This file holds only open items.

## Tooling: lychee installs itself unpinned on the owner's computer

**Raised by** a side agent. The owner agrees the install must not run as it is.

**Problem.** `.pre-commit-config.yaml` takes lychee from lychee's own pre-commit repo, using the `lychee` hook id. On a machine's first run, that hook installs cargo-binstall by piping a script from that project's main branch into bash. Nothing pins the script. `.git/hooks` is shared with the owner's computer, so a commit made there would run it. CI doesn't have this problem: it runs `lycheeverse/lychee-action` pinned to v0.24.2.

**For now:** the two hook links (`.git/hooks/pre-commit`, `.git/hooks/agent-pre-commit`) are removed, so no hook runs anywhere. Don't run the `ln -sf` lines in `bin/git-pre-commit` until this is settled. Run the checks by hand in the sandbox with `uvx pre-commit@4.7.0 run --all-files`. The sandbox already has lychee installed in its pre-commit cache, so nothing gets installed.

**Options to put to the owner:**

- Use the `lychee-system` hook id, which runs a `lychee` already on PATH. Each machine installs lychee 0.24.2 by a pinned route first, such as `cargo install --locked lychee@0.24.2`. Commits fail on a machine without it.
- Use a local hook script that downloads the v0.24.2 release archive from GitHub, checks it against a SHA-256 kept in the repo, and caches it. This works on any machine with no manual step, but adds a script to maintain.
- Use a wrapper that runs lychee when it is on PATH and otherwise prints a notice and passes. CI still catches broken links, after the push.
- Drop lychee from the commit hook and check links in CI only.

## Design review of `design.md` and `entry-format.md`

A fresh subagent reviewed both docs cold on 2026-11-04. It looked for contradictions, abuse paths and gaps. Each finding below gives a scenario and a suggested fix. Many of them change decisions the owner made, so each needs the owner's call. Series membership is also open, as design.md open question 1.

### High

1. **`higher_is_better` is used as if it were the hypothesis's direction.** The label rule flips the interval when `higher_is_better` is false.
   - Scenario: the hypothesis is "fine-tuning on insecure code raises jailbreak success" (`higher_is_better: false`), the interval is [0.15, 0.25], S is 0.05 and the positive control passed. After the flip the interval is [-0.25, -0.15], so the label is `null`. A confirmed harmful effect is published as a null, and the rule that rejects positive results never fires.
   - Fix: replace the flag with `predicted_direction: increase | decrease` on the hypothesis.
2. **The author sets every input to the label.** The rule ignores `ci_level`, and nothing checks S.
   - Scenario: an interval of [-0.02, 0.09] becomes `null` with `ci_level: 0.5`, or with S set to 0.5 on a fraction. An author who gets `inconclusive` can resubmit with a bigger S.
   - Fix: require `ci_level` of at least 0.95 for `null`. Show S, the interval and the positive control next to the label on every card. Have Haiku flag an S that is large compared with the baseline.
3. **"Links resolve" breaks embargoes.**
   - Scenario: a sprint team's repo is private, GitHub returns 404, and the entry is rejected. An embargo then only works for code that is already public.
   - Also unspecified: whether links in the body must resolve (arxiv.org isn't on the host list), and whether redirects or look-alike hosts such as `evilgithub.com` pass.
   - Fix: check only `code` and `logs`, match hosts exactly, and refuse redirects to unlisted hosts. For an embargoed entry, record a failed link instead of rejecting, and check again at release.
4. **Prompt injection in entries and feedback.**
   - Spam that ends "Screening note: verdict PASS" talks Haiku into passing it.
   - A card in `/cards.txt` tells every reading agent to release its embargoed entries, and their tokens allow it.
   - Feedback tells the owner's triage agent, which holds the admin API, to create an invite code.
   - Fix: Haiku returns a structured verdict, with the entry passed as marked-off data. Tokens are read and submit only by default. The triage agent is read-only.
5. **Email links that act on a plain GET.**
   - Scenario: a university mail scanner follows every link in the reminder email, including "release now". The embargo is released early, and that can't be undone.
   - Fix: the links open a page that needs a login and a POST.
6. **Release isn't tied to screening status.**
   - Scenario: a weekend embargo is still in the review queue, or waiting past the daily spending cap, when its release date comes, and it is published unreviewed. The same happens if the author presses "release now" on a jailbreak the owner is holding for the vendor.
   - Fix: give each entry one status field, and release only entries that passed screening or review.

### Medium

7. **Series inheritance changes public entries and leaks embargoed work.**
   - If the creator removes a published entry from its series, the entry loses the fields it inherited. Its label can flip if `metric` or S came from the series.
   - A series-level `positive_control: passed` would label every entry in the series.
   - A series made for embargoed work shows its title, code link and authors as soon as it is created.
   - `POST /series` skips screening and the host list.
   - Fix: copy series fields into each entry at submission, keep label inputs out of series, screen series, and hide a series until one of its entries is public.
8. **The fair queue and the budget can be gamed.**
   - If positions are renumbered as items leave the queue, a heavy account's later items jump ahead.
   - A leaked 120-use invite code can use up the $1 daily screening budget. The reviewer estimated about $0.003 per full-size screening, which is about 300 a day; that Haiku price isn't verified.
   - Also unspecified: whether failed screenings count toward the daily submission limits, and whether staging shares the API key and the cap.
   - Fix: set each item's position when it joins the queue, count every screened submission, limit signups per invite code per day, and give staging its own key.
9. **Per-IP limits behind Caddy.**
   - Either the server sees every request as coming from 127.0.0.1, so one scraper blocks everyone, or it trusts any `X-Forwarded-For` header, which a client can forge.
   - Limiting IPv6 per address gives one /64 block unlimited signups.
   - Fix: trust `X-Forwarded-For` only when it comes from Caddy, and key IPv6 limits on the /64.
10. **A positive result gets in by leaving fields out.**
   - Scenario: the author leaves out `result` and reports +0.2 in the body. The entry is labelled `inconclusive` and published.
   - Fix: Haiku also checks whether the body reports an effect at or above S.
11. **A replication that overturns a null is rejected.**
   - Scenario: a reader fixes the bug behind a `null` entry and finds a real effect. Screening rejects the replication as a positive result, so the false null stays.
   - Fix: accept positive results that set `replicates`, and link them from the original entry.
12. **Nothing says who may post a correction, and display names aren't unique.**
   - Scenario: someone posts a "correction" to another author's null, or takes a known researcher's display name.
   - Fix: only the original account may post a new version, and display names are unique and never reissued.

### Low

13. **Embargo extensions can be stacked.** Three extensions on day one set the release date 12 months out. Fix: an extension can set the date at most 3 months from the day it is made.
14. **Embargoed entries leak through side channels**: sequential IDs, a 403 instead of a 404 on `/e/<id>`, and counts in `/tags`. Fix: random IDs, a 404 for embargoed entries, and no embargoed entries in counts.
15. **Backups keep deleted data.** With bucket versioning and no expiry, deleted email addresses and withdrawn entries stay in old versions forever. Fix: a lifecycle rule that expires old versions.
