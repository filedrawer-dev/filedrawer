# Prior art

Surveyed on 2026-11-03. I found no hosted archive of null results that is focused on AI safety and built for agents to read. Search again before claiming that File Drawer is new.

## Closest

- [Negative Result Repository](https://t46.github.io/blogs/negative_result_repository.html) (Shiro Takagi, 2026; code at `t46/negative-result-repository`). A prototype that stores failed experiments from automated research loops. Its `check_proposal()` shows similar past failures before a new run. It serves one user, isn't hosted and isn't about safety. In spirit it is closest to File Drawer's main reader, an agent checking a planned experiment.
- [Nullary](https://glama.ai/mcp/servers/nullary-ai/nullary-mcp). About 15.6M drug-discovery negative results that agents query over MCP. The same "nulls for agents" idea in another field.

## Preprint servers for papers written by AI

None of them focuses on nulls. They are aiXiv, AiraXiv (which offers MCP access), clawXiv, ai-archive.io, AgentRxiv, JAIGP, ai.vixra.org and rxiVerse. Martin Monperrus keeps a [list](https://www.monperrus.net/martin/preprint-servers-of-AI-generated-papers).

## Proposals that built nothing

- [Aaron Scher, LessWrong shortform](https://greaterwrong.com/posts/gJyGQrGrojNnCQZgD/aaron_scher-s-shortform/comment/rMxXPJyhKEQC3mNRp), 2023-09-04. He proposed a pre-registration database for AI safety experiments that would include null results, then judged it not worth building. His reasons were write-up cost, reviewer time, low prestige, coordination, info hazards, and private work that would never enter the registry. Agents remove most of the write-up cost. The other objections still apply.
- Sungmin Lee, [LLMs Have Made Failure Worth Publishing](https://arxiv.org/abs/2604.06236), April 2026. Argues that LLMs make publishing failures worth the effort. Builds nothing.
- Kargi Chauhan, [Dead Science Walking](https://arxiv.org/abs/2606.04220), June 2026. Recommends null-result databases as infrastructure for AI scientists. Among its failure modes are "replication laundering" and "confident rediscovery". Bad nulls in an archive feed both of these, which is why File Drawer has its label rule.

## Venues for people

- I Can't Believe It's Not Better (ICBINB), a workshop on negative results. Its ICLR 2026 edition covered where LLMs fall short.
- [Null Results Wiki](https://github.com/jt0mkins/null-results-wiki). A static site covering all fields. On 2026-11-03 it had 0 stars and only example entries.
- [My Failed AI Safety Research Projects](https://www.boristhebrave.com/2025/06/19/my-failed-ai-safety-research-projects-q1-q2-2025/) (Boris the Brave, 2025). The kind of content File Drawer wants, published as a blog post.

## Existing embargo mechanisms

- OSF registrations can be embargoed for up to 4 years and then become public automatically.
- Zenodo opens embargoed files automatically on the end date. The record's metadata stays visible during the embargo.

## Queries run

- null results repository AI safety experiments negative results database
- preprint server for AI agents research "by agents" null results 2026
- machine learning negative results archive embargo auto-release experiment logs
- Alignment Forum "negative results" tag interpretability null result posts
- LessWrong "null results" OR "negative results" AI safety post repository proposal file drawer
- "I Can't Believe It's Not Better" workshop negative results 2026
- open registry of failed alignment interpretability experiments agents can query MCP 2026

No search was run on LessWrong itself.
