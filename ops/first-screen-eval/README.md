# The first screen's cases

`ops/first-screen-bakeoff` (`-h`) sends these to Jev, the cheap model #131 would put in front of Haiku, and counts what a rule would reject.
Every case is made up.
They are `.txt` so no markdown check reads them: junk is junk on purpose.

- `tune/` and `heldout/` are the two splits.
  Questions and the rule are chosen on `tune/` and committed before the one run on `heldout/`, which the script enforces.
  A separate agent wrote `heldout/` without seeing `tune/`, the questions or the rule, so the held-out run tests more than the author's own idea of junk.
- `<split>/templates/`: whole entries with one `{{BODY}}` line, the header a junk submitter would wrap content in to get past the server's mechanical checks.
- `<split>/junk/`: junk bodies, each put under every template of its split.
  A body counts as rejected only when it is rejected under all of them.
- `<split>/junk-entries/`: whole junk entries whose header describes them truthfully, such as a shopping list titled as one.
  A header that matches its body is the plainest file-store use, and the templates can't show it.
- `<split>/honest/`: whole honest entries that look like junk at a glance.
  None may be rejected.
- `<split>/honest-edge/`: honest AI safety entries where no model is the object of the experiment, such as a simulation or a survey of annotators, which a question about AI is most likely to get wrong.
- `<split>/hand-on/`: filler dressed as AI safety research, which is Haiku's to judge; reported, scored as neither.
- `questions.json`: the model and what it is asked.
- `rule.json`: when an answer rejects.

No case may start with `#!`, hold encoded blobs, which the server refuses before any screen, or be refused by `entry-check` once composed.
The scores go in the workspace's `keep/filedrawer/first-screen/`, outside this repo, since they name the local field-test entries the honest baseline comes from.
