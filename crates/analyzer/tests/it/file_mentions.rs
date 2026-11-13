//! Seam: `file_mentions`, the files an entry's body names in code spans, which the
//! server checks against its attachment set (design.md, Agent interface).

use crate::common;

use common::entry;
use filedrawer_analyzer::file_mentions;

fn mentions(body: &str) -> Vec<String> {
    file_mentions(&entry("", body))
}

#[test]
fn code_spans_that_are_one_file_path_are_named_cleaned_and_once() {
    assert_eq!(
        mentions(
            "Recorded in `research/plan-collusion-controls.md` and `P-sim/logit_nash.py`, \
             which `P-sim/logit_nash.py` prints; see `./scripts/run.py:42`, \
             `author:analysis/compute.py`, `apart-sprint@696f838:notes/S.md`, \
             `../other-repo/paper/refs.bib`, `/home/me/x.sh`, `~/notes.txt` \
             and `judge.py@abc1234`."
        ),
        vec![
            "research/plan-collusion-controls.md",
            "P-sim/logit_nash.py",
            "scripts/run.py",
            "analysis/compute.py",
            "notes/S.md",
            "other-repo/paper/refs.bib",
            "home/me/x.sh",
            "notes.txt",
            "judge.py",
        ]
    );
}

#[test]
fn data_logs_locks_globs_urls_and_dotted_names_aren_t_files_to_attach() {
    assert_eq!(
        mentions(
            "Not `results.jsonl`, `runs.csv`, `out.json`, `run.log`, `uv.lock`, \
             `data/blind-cartel-data.tar.xz`, `results_*.jsonl`, `https://x.org/a.py`, \
             `np.mean`, `meta-llama/Llama-3.1-8B`, `gpt-4.1`, `python3 run.py`, \
             `slug:baseline` or `x.PY.bak`."
        ),
        Vec::<String>::new()
    );
}

#[test]
fn only_inline_spans_in_the_body_count() {
    let text = entry(
        "hypothesis_source: notes/plan.md@abc1234\n",
        "A link [to `linked.py`](https://github.com/a/b/blob/0123456/linked.py) opens it.\n\n\
         ```\npython3 fenced.py\n```\n\n    indented.py\n",
    );
    assert_eq!(file_mentions(&text), Vec::<String>::new());
}
