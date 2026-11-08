# Prior art

Surveyed on 2026-11-03 for null and inconclusive results.
Widened on 2026-11-04 to any AI safety result that won't become a paper, including positive effects nobody plans to refine.
I found no hosted venue that takes such results from anyone, labels each by outcome, and lets an agent check a planned experiment against them.
Many venues hold the results themselves, such as Apart's sprint archive, LessWrong and the labs' notes series.
They hold them as papers or prose, with no outcome label and no fields an agent could match a planned experiment against.
Search again before claiming that File Drawer is new.

## Closest

- [Apart Research sprint projects](https://apartresearch.com/sprints/projects).
  Every project submitted to Apart's weekend sprints: 2,022 across 34 sprints from September 2022 to October 2026, by the count in its project index on 2026-11-04.
  Each page has an abstract, a PDF report, judges' scores and critiques, and sometimes a repo link (6 of 11 sampled pages).
  Each page calls the work "early-stage".
  Some titles say null or inconclusive, such as "Rotating verifier partners in DeepSeek-V4-Flash: an inconclusive 8-point drop in acceptance".
  Agents can read the whole index through [`/sprints/projects/llms.txt`](https://apartresearch.com/sprints/projects/llms.txt).
  The site search covers titles only, and I found no API.
  Closest in content, but each result is a PDF paper scored for prizes, with no outcome label and no code pinned to a commit.
- [LessWrong](https://www.lesswrong.com) and the [Alignment Forum](https://www.alignmentforum.org).
  Many unrefined AI safety results are posted here.
  On 2026-11-04 LessWrong held about 55,000 posts, 16,166 of them tagged AI and 4,700 also on the Alignment Forum, counted through its GraphQL API.
  No tag marks null or negative results.
  Only 3 AI posts have "negative result(s)" or "null result(s)" in the title, such as [Negative Results on Group SAEs](https://www.lesswrong.com/posts/jKKbRKuXNaLujnojw/negative-results-on-group-saes).
  More say "failed replication", "research note", "initial results" or "progress update".
  Agents can read and search both sites through a Markdown API described at [`/api/SKILL.md`](https://www.lesswrong.com/api/SKILL.md), or through GraphQL.
  Third-party MCP servers wrap them, such as [lesswrong-mcp](https://github.com/seankessinger/lesswrong-mcp) and [mcp-alignmentforum](https://github.com/rapturt9/mcp-alignmentforum).
  Results have no outcome label or experiment fields.
  The site is written for people.
  Its [2025 LLM policy](https://www.lesswrong.com/posts/KXujJjnmP85u8eM6B/policy-for-llm-writing-on-lesswrong) let an autonomous agent post only as coauthor with a human who vouches for the result.
  A [March 2026 update](https://www.lesswrong.com/posts/nQWavk9mnwcv6ScMR/new-lesswrong-editor-also-an-update-to-our-llm-policy) replaced it, and automatically rejects posts that score above a threshold in its LLM-content detector.
- [Negative Result Repository](https://t46.github.io/blogs/negative_result_repository.html) (Shiro Takagi, 2026; code at `t46/negative-result-repository`).
  A prototype that stores failed experiments from automated research loops.
  Its `check_proposal()` shows similar past failures before a new run.
  It serves one user, isn't hosted and isn't about safety.
  On 2026-11-04 its repo described itself as kept for the record and unmaintained.
  In spirit it is closest to File Drawer's main reader, an agent checking a planned experiment.
- [Null Registry](https://nullregistry.org) (August 2026).
  "A public commons of negative results and dead ends for AI agents and humans", so that an agent can "check what already failed before spending effort rediscovering it".
  Agents read it through an llms.txt and a JSON index of every record, and submit by pull request.
  Each record states how far it transfers, with a confidence that rises only through independent reproduction.
  Its index held 31 records on 2026-11-04, mostly software and math dead ends, none about safety.
  Closest in mechanism, but it holds gotchas rather than research results, with no code or logs.
- [Nullary](https://nullary.ai).
  Over 122M drug-discovery negative results by its own [count](https://nullary.ai/llms.txt) on 2026-11-04, which agents query over [MCP](https://glama.ai/mcp/servers/nullary-ai/nullary-mcp) or a REST API.
  The same "nulls for agents" idea in another field, built from public databases rather than from researchers' own submissions.

## Venues for unrefined AI safety results

### LessWrong formats

- [Quick takes](https://www.lesswrong.com/quicktakes).
  Short posts on a user's shortform page.
  There were 16,018 from 2018 to 2026-11-04, and 4,613 of them in 2026.
  Since January 2026 most carry topic tags, and 3,031 of 2026's are tagged AI (GraphQL counts).
  Some are one-off experiments, such as [Fabien Roger's on weight steering against training gamers](https://www.lesswrong.com/posts/nAsMfmxDv6Qp7cfHh/fabien-s-shortform?commentId=jcbYcoDzkkwYtppSx).
  Agents search them as comments through the same API.
  They are unstructured notes found only by keyword.
- Tags.
  [Research Agendas](https://www.lesswrong.com/w/research-agendas) (275 posts by the tag's count) holds plans, not results.
  [Postmortems & Retrospectives](https://www.lesswrong.com/w/postmortems-and-retrospectives) (234) is mostly about programs and organisations, not experiments.
  [Experiments](https://www.lesswrong.com/w/experiments) (89) mixes AI and other topics.
  Program tags collect some project write-ups: [MATS Program](https://www.lesswrong.com/w/mats-program) 398, AI Safety Camp 121, Apart Research 58, SPAR Program 3.
  Tags group posts by topic or program, never by outcome.

### Labs' notes series

Each of these is one organisation's prose, chosen by its staff.
None takes outside submissions or has outcome labels.
Counts are from each series' index on 2026-11-04.

- Anthropic, [Circuits Updates](https://transformer-circuits.pub/).
  20 monthly updates from May 2023 to October 2026, plus 3 standalone update pages.
  Each update includes "minor points we wish to share, since we're unlikely to ever write a paper about them".
  Some are nulls, such as a failed attempt to replicate JumpReLU's gains ([August 2024](https://transformer-circuits.pub/2024/august-update/index.html)).
  Atom feed, no search.
  A null is a paragraph inside a monthly digest.
- Anthropic, [Alignment Science Blog](https://alignment.anthropic.com/).
  65 posts since 2024, for "research notes and early findings that we don't think warrant a full publication".
  One is [Fine-Tuned Lie Detectors Failed to Generalize](https://alignment.anthropic.com/2026/lie-detectors/).
  No feed found.
  It also carries companion posts for papers.
- OpenAI, [Alignment Research Blog](https://alignment.openai.com/).
  25 posts of its own since 2025-12-01.
  Its [launch post](https://alignment.openai.com/hello-world/) calls it "a lab notebook" for "work that otherwise wouldn't have been published".
  It posts nulls, such as [How far does alignment midtraining generalize?](https://alignment.openai.com/how-far-does-alignment-midtraining-generalize/).
  RSS feed.
  The closest lab series in intent.
- Google DeepMind's mech interp team, [progress updates](https://www.alignmentforum.org/s/AtTZjoDm8q3DbDT8Z).
  An 11-post Alignment Forum sequence, of which 2 are progress updates of snippets "that didn't meet our bar for a full paper".
  The second is [Negative Results for SAEs On Downstream Tasks](https://www.lesswrong.com/posts/4uXCAJNuPKtKBsi28/negative-results-for-saes-on-downstream-tasks).
  Readable through the LessWrong API.
- [METR Notes](https://metr.org/notes/).
  19 notes from August 2025 to September 2026, "Rough/unpolished research updates and speculation".
  Full-text RSS feed.
  Mixed with essays.
- [Apollo Research](https://www.apolloresearch.ai/science) research notes, "less rigorous than our standard for a published paper".
  At least 2, including one finding that its scheming precursor evals had limited predictive power.
  Sitemap only.
- [EleutherAI research notes](https://blog.eleuther.ai/categories/research-notes/).
  15 posts, including a retrospective whose effect "was not statistically distinguishable from chance".
  Sitemap only.
- I found no notes series for unrefined or null results at Redwood Research, Goodfire, UK AISI, FAR.AI or Palisade Research.
  I checked their index pages, feeds and sitemap slugs.

### Sprint and fellowship outputs

- [SPAR Research Library](https://library.sparai.org/).
  143 "working reports" from Spring 2025 to Spring 2026 that "may not represent final research conclusions".
  Each page has its abstract in JSON-LD, and there is a sitemap but no search API.
  A selection, not every project.
- [AI Safety Camp](https://www.aisafety.camp/) research outputs.
  One Google Sites page per edition, listing every team's project, including some with no output.
  Roughly 100 to 130 projects across 9 edition pages.
  No API or sitemap.
  Each project is a paragraph that links out.
- MATS ([248 research pages](https://www.matsprogram.org/research)), LASR Labs (27), Pivotal Research (35, with an llms.txt), ERA (about 10), CBAI (17) and Algoverse list finished or accepted papers only. BlueDot (7 project posts) and ARENA (38 selected capstones) show curated course projects. <!-- slop-lint: ignore -->
- aisafetyideas.com, a board of research ideas made by Apart in 2023, is now a research-bounty site with no ideas posted.

### Open collaborations

- [ARBOR](https://arborproject.github.io/).
  An open collaboration on interpreting reasoning models, started in February 2025, that shares "partial progress and early results" to "eliminate duplicative work".
  Projects are [GitHub Discussions](https://github.com/ARBORproject/arborproject.github.io/discussions): 16 on 2026-11-04, most from February and March 2025.
  Readable through GitHub's API.
  One topic, and mostly dormant.

## Agent-readable AI safety indexes

- [StampyAI Alignment Research Dataset](https://huggingface.co/datasets/StampyAI/alignment-research-dataset).
  14,271 documents from arXiv, LessWrong, the Alignment Forum, blogs and other sources, as bulk JSONL.
  The Hugging Face copy was last updated in November 2023, though its scraper still runs.
  Published writing, with no outcome labels.
- [AISafety.com](https://aisafety.com/llms.txt) and [grantmaking.ai](https://grantmaking.aisafety.com/llms.txt) have llms.txt files, and grantmaking.ai has a JSON API over 1,883 projects.
  They are directories of organisations, people and funding, not results.

## Built for agents, outside AI safety

- [knoten](https://github.com/BY571/knoten) (2026).
  A per-team research graph kept as Markdown in git, which agents read through a CLI with JSON output.
  A dead hypothesis records its cause from a fixed list (such as `no_signal` or `underpowered`), a repro command, and what would reopen it.
  There is no public corpus, only two example graphs.
  A notebook for one project, but its record format is worth comparing during the field test.
- Agora (NVIDIA), [Git as Shared Memory for Collective AutoResearch](https://arxiv.org/abs/2609.18094), September 2026.
  Agents in one project commit results, hypotheses, verifications and negative results to an append-only graph in git.
  One 12-day run had 13 LLM workers and 1,703 contributions.
  I found no public instance or code.
  Memory inside one project, not an archive across projects.
- [autoresearch-at-home](https://github.com/mutable-state-inc/autoresearch-at-home) (March 2026) publishes every experiment, success or failure, with its full source to a shared workspace, for one benchmark and one metric.
  [Negative Knowledge](https://github.com/hch-wang/Negative_Knowledge) (May 2026) turns failed attempts into typed records in one project's memory, with no public corpus.
- [Open Negative Results Registry](https://github.com/Ciprian-LocalPulse/open-negative-results-registry) (July 2026).
  Medical nulls as JSON files under a schema, with a static search site.
  It had 30 files on 2026-11-04, examples included.
- [Weights & Biases](https://github.com/wandb/wandb-mcp-server) and [Hugging Face](https://huggingface.co/mcp) run official MCP servers.
  W&B's reads one team's runs and reports and needs an API key.
  Hugging Face's reads models, datasets and papers, with no notion of what a finding was.
  [OpenML](https://www.openml.org) holds over 10M benchmark runs, whatever their outcome, behind a REST API.
  None of them records what was learned.

## Venues for small or null results in other fields

- [ClinicalTrials.gov](https://clinicaltrials.gov).
  610,285 registered studies on 2026-11-04, 80,881 of them with results posted whatever the outcome.
  A trial stopped early carries the reason.
  It has a REST API with no key and third-party MCP servers such as [clinicaltrialsgov-mcp-server](https://github.com/cyanheads/clinicaltrialsgov-mcp-server).
  It shows what mandated, outcome-blind reporting in a fixed schema produces, for clinical trials only.
- [Octopus](https://www.octopus.ac).
  Splits research into eight linked publication types, so a result can be published on its own.
  28,640 publication versions, most of them bulk imports and 610 by individuals.
  Its JSON read API is open but undocumented: this survey found it in the source code.
  Human-written, all fields, no null label.
- [microPublication Biology](https://www.micropublication.org/journals/biology).
  Peer-reviewed single findings, some tagged "negative result".
  About 1,990 articles in PubMed Central, read through NCBI or Europe PMC; I found no API of its own.
- [ReScience C](https://rescience.github.io).
  224 replication reports, 6 of them titled `[¬Re]` or `[¬Rp]` to mark a failed replication.
  Readable as one bib file or through Zenodo's API.
  A precedent for a machine-readable "failed" marker.
- [Journal of Trial and Error](https://journal.trialanderror.org) (129 articles, readable through Crossref), PLOS ONE's [Missing Pieces](https://collections.plos.org/collection/missing-pieces/) collection (41 articles, the latest from 2021), [F1000Research](https://f1000research.com) (accepts nulls without labelling them; Solr API) and [ResearchEquals](https://www.researchequals.com) (350 modules; OpenAPI).
  Journals and publishers written for people.
- Zenodo, OSF and Figshare have no community or collection for negative results that I could find.
  On Zenodo about 1,200 records carry a keyword such as "negative results" or "null result".

## Preprint servers for papers written by AI

None of them focuses on nulls, though AiraXiv's [paper](https://arxiv.org/abs/2605.21481) says it encourages negative results.
They are aiXiv, AiraXiv (which offers MCP access), clawXiv, ai-archive.io, AgentRxiv, JAIGP, ai.vixra.org and rxiVerse.
Martin Monperrus keeps a [list](https://www.monperrus.net/martin/preprint-servers-of-AI-generated-papers).

## Proposals that built nothing

- [Aaron Scher, LessWrong shortform](https://greaterwrong.com/posts/gJyGQrGrojNnCQZgD/aaron_scher-s-shortform/comment/rMxXPJyhKEQC3mNRp), 2023-09-04.
  He proposed a pre-registration database for AI safety experiments that would include null results, then judged it not worth building.
  His reasons were write-up cost, reviewer time, low prestige, coordination, info hazards, and private work that would never enter the registry.
  Agents remove most of the write-up cost.
  The other objections still apply.
- Sungmin Lee, [LLMs Have Made Failure Worth Publishing](https://arxiv.org/abs/2604.06236), April 2026.
  Argues that LLMs make publishing failures worth the effort.
  Builds nothing.
- Kargi Chauhan, [Dead Science Walking](https://arxiv.org/abs/2606.04220), June 2026.
  Recommends null-result databases as infrastructure for AI scientists.
  Among its failure modes are "replication laundering" and "confident rediscovery".
  Bad nulls in an archive feed both of these, which is why File Drawer has its label rule.
- Michael Lan et al., [Make Mechanistic Interpretability Auditable](https://arxiv.org/abs/2606.00033), ACL 2026.
  Proposes a community platform for results that fit outside a paper: negative results, reproductions, partial results and "minor yet useful results which are not enough for a paper".
  I found no sign that it was built.
- Jason Fantl, [Autonomous researchers will strain peer review, funding, and oversight](https://www.lesswrong.com/posts/WgroAK7N3jAfBBpct/autonomous-researchers-will-strain-peer-review-funding-and), LessWrong, 2026-10-21.
  Expects AI researchers to worsen the file drawer problem, and calls for a metascience organisation for AI safety to build new publishing infrastructure.
  Builds nothing.

## Venues for people

- I Can't Believe It's Not Better (ICBINB), a workshop on negative results.
  Its ICLR 2026 edition covered where LLMs fall short.
- [Null Results Wiki](https://github.com/jt0mkins/null-results-wiki).
  A static site covering all fields.
  On 2026-11-03 it had 0 stars and only example entries.
- [My Failed AI Safety Research Projects](https://www.boristhebrave.com/2025/06/19/my-failed-ai-safety-research-projects-q1-q2-2025/) (Boris the Brave, 2025).
  The kind of content File Drawer wants, published as a blog post.

## Existing embargo mechanisms

- OSF registrations can be embargoed for up to 4 years and then become public automatically.
- Zenodo opens embargoed files automatically on the end date.
  The record's metadata stays visible during the embargo.

## Queries run

On 2026-11-03, web searches:

- null results repository AI safety experiments negative results database
- preprint server for AI agents research "by agents" null results 2026
- machine learning negative results archive embargo auto-release experiment logs
- Alignment Forum "negative results" tag interpretability null result posts
- LessWrong "null results" OR "negative results" AI safety post repository proposal file drawer
- "I Can't Believe It's Not Better" workshop negative results 2026
- open registry of failed alignment interpretability experiments agents can query MCP 2026

On 2026-11-04, LessWrong and the Alignment Forum through their own APIs:

- Tag list (GraphQL, 1,016 tags) grepped for: negative, null, agenda, postmortem, retrospect, replicat, log, progress, update, preliminar, fail, notebook, work in progress, shortform, quick, sprint, hackathon, research note, open problem, empirical, exploratory, inconclusive, result, experiment, reproduc, unpublished, draft.
- Tag slugs looked up: negative-results, null-results, replication, replications, research-log, research-logs, lab-notebook, shortform, quick-takes, work-in-progress, preliminary-results, failed-experiments, failure, research-updates, progress-update, empirical.
  Only work-in-progress exists, as a wiki page with no posts.
- Tag search: negative; null; negative results; failure; failed; replication; research notes; research log; preliminary; inconclusive; work in progress; experiments; empirical results.
- Post search: "file drawer"; "negative results" AI safety publish; "null results" alignment; database of failed experiments; registry of AI safety experiments; unpublished results interpretability; what we tried that did not work; research log; abandoned project interpretability.
- Quick take search: "negative result"; "null result"; "didn't work" SAE; "inconclusive"; quick experiment steering; "failed to replicate".
- Titles of all 54,963 posts, matched against: negative or null result(s); didn't, doesn't or did not work, fail, failed, failure; inconclusive, mixed results; preliminary, early, initial or partial with results, findings or experiments; research note, log or notebook; progress, research or interim update or report; WIP, rough notes, quick results; replicate, replication; abandoned, postmortem.

On 2026-11-04, web searches:

- LessWrong MCP server Alignment Forum posts search agents
- database of AI safety experiments already tried "has this been tried" interpretability results tracker
- platform for publishing small unpolished AI alignment research results "research notes" 2026
- "Make Mechanistic Interpretability Auditable" collaborative reviewing platform launched
- ICBINB workshop ICLR 2026 "I Can't Believe It's Not Better" theme
- AiraXiv MCP preprint server AI-generated papers
- GDM Mech Interp Team Progress Update alignmentforum
- Athena AI safety mentorship program women research outputs
- "AI Safety Hub Labs" renamed LASR Labs
- AI safety hackathon negative results archive projects null results database
- MCP server AI safety alignment research papers search LessWrong Alignment Forum
- MCP server "negative results" research database
- "failed experiments" database API agents query MCP
- octopus.ac API documentation publications endpoint
- OpenML MCP server
- Weights & Biases MCP server public reports query
- ClinicalTrials.gov MCP server results database
- MCP server "lab notebook" OR "research log" experiments agents share results
- "experiment registry" API null results machine learning agents
- database of AI safety research projects null results queryable API 2026
- autoresearch agents shared database of tried ideas "dead ends" open source 2026
- llms.txt "negative results" OR "null results" archive
- hackathon research projects archive AI safety searchable API Apart sprint projects dataset

On 2026-11-04, GitHub repository searches: alignment research dataset mcp; lesswrong mcp; alignment forum mcp; ai safety mcp server papers; stampy mcp; negative results mcp; null results mcp; failed experiments mcp; negative results database api; null result registry.

On 2026-11-04, each venue's own pages were also checked for `/llms.txt`, `/robots.txt`, `/sitemap.xml`, RSS or Atom feeds and JSON endpoints, and searched where it had a search box or API:

- MATS site search: negative; null; inconclusive; does not; fails.
- Apart's project index: null result; negative result; inconclusive; no significant; did not; failed to; does not.
- Octopus: null result; negative result; no significant; language model.
- Zenodo communities: negative results; null results; failed experiments; negative; reproducibility.
  Zenodo record keywords: "negative results"; "negative result"; "null results"; "null result".
- Figshare collections: "negative results"; "null results".
- F1000Research titles and abstracts: negative result(s); null result(s).
- Hugging Face datasets: null-result; negative-results.
- PLOS, PubMed Central and DataCite: titles or full text with negative or null result(s).
