"""Tests for ops/first-screen-bakeoff (#131): which cases it builds, what it sends
to the API and keeps of the answers, and what its report counts. The API is a local
fake behind FIRST_SCREEN_API_URL; nothing here reaches the network."""

import http.server
import importlib.machinery
import importlib.util
import json
import subprocess
import threading
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
KEY = "test-key-never-printed"
MODEL = "jev-9.9.9"
ENTRY = "---\ntitle: t\n---\n\n## Setup\n\ntext\n"


def load():
    loader = importlib.machinery.SourceFileLoader(
        "first_screen_bakeoff", str(REPO / "ops/first-screen-bakeoff")
    )
    spec = importlib.util.spec_from_loader("first_screen_bakeoff", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class FakeApi:
    """A stand-in for the API. `answer(state)` returns the probabilities for one
    request, or a (status, headers, body) reply; `requests` holds each body sent."""

    def __init__(self):
        self.requests = []
        self.auth = []
        self.answer = lambda state: {"junk": 0.5}
        self.model = MODEL
        self.lock = threading.Lock()
        fake = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                with fake.lock:
                    fake.requests.append(body)
                    fake.auth.append((self.path, self.headers.get("Authorization")))
                    reply = fake.answer(body["state"])
                if isinstance(reply, dict):
                    answers = {
                        name: {"type": "noul", "noul": p} for name, p in reply.items()
                    }
                    reply = (
                        200,
                        {},
                        {
                            "model": fake.model,
                            "answers": answers,
                            "usage": {"input_tokens": 100, "output_tokens": 3},
                        },
                    )
                status, headers, payload = reply
                data = json.dumps(payload).encode()
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def states(self):
        return [body["state"] for body in self.requests]


@pytest.fixture
def api(monkeypatch):
    fake = FakeApi()
    monkeypatch.setenv("FIRST_SCREEN_API_URL", fake.url)
    monkeypatch.setenv("JEV_API_KEY", KEY)
    monkeypatch.setenv("FIRST_SCREEN_BACKOFF_SECONDS", "0")
    yield fake
    fake.close()


def git(folder, *argv):
    subprocess.run(["git", "-C", str(folder), *argv], check=True, capture_output=True)


class World:
    """A temp git repository standing in for the case folders, with the questions
    and a rule committed, an ignored folder for the scores, and a runner."""

    def __init__(self, tmp_path):
        self.tmp = tmp_path
        self.cases = tmp_path / "first-screen-eval"
        self.screen_eval = tmp_path / "screen-eval"
        self.field_test = tmp_path / "field-test"
        self.out = tmp_path / "ignored" / "out"
        for folder in (self.cases, self.screen_eval, self.field_test):
            folder.mkdir()
        git(tmp_path, "init", "-q")
        git(tmp_path, "config", "user.email", "t@example.invalid")
        git(tmp_path, "config", "user.name", "t")
        (tmp_path / ".gitignore").write_text("ignored/\nfield-test/\nentry-check\n")
        git(tmp_path, "add", ".gitignore")
        self.questions = tmp_path / "questions.json"
        self.rule = tmp_path / "rule.json"
        self.rule.write_text(
            json.dumps({"reject_if_any": [{"question": "junk", "at_least": 0.9}]})
        )
        self.set_questions({"junk": {"type": "noul", "instructions": "Is it junk?"}})
        self.entry_check = tmp_path / "entry-check"
        self.entry_check.write_text(
            '#!/bin/sh\nif grep -q REFUSEME "$1"; then exit 1; fi\nexit 0\n'
        )
        self.entry_check.chmod(0o755)
        self.module = load()

    def set_questions(self, questions, model=MODEL, commit=True):
        self.questions.write_text(json.dumps({"model": model, "questions": questions}))
        if commit:
            self.commit()

    def commit(self):
        git(self.tmp, "add", "questions.json", "rule.json")
        git(self.tmp, "commit", "-q", "--allow-empty", "-m", "questions and rule")

    def write(self, path, text=ENTRY):
        path = self.tmp / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def folders(self):
        return [
            "--cases",
            str(self.cases),
            "--screen-eval",
            str(self.screen_eval),
            "--field-test",
            str(self.field_test),
        ]

    def run(self, *extra, split="all", draws="1", guard=True):
        """Runs `run`. Past the tune split it passes the rule and the entry check a
        held-out run requires, unless `guard` is False."""
        argv = ["run", "--questions", str(self.questions), "--out", str(self.out)]
        argv += ["--split", split, "--draws", draws, *self.folders(), *extra]
        if guard and split != "tune":
            argv += ["--rule", str(self.rule), "--entry-check", str(self.entry_check)]
        return self.module.main(argv)

    def report(self, *extra):
        return self.module.main(["report", str(self.out), *extra])

    def rows(self):
        lines = (self.out / "scores.jsonl").read_text().splitlines()
        return {row["id"]: row for row in map(json.loads, lines)}


@pytest.fixture
def world(tmp_path):
    return World(tmp_path)


def test_field_test_entries_are_honest_and_notes_are_left_out(world, api):
    world.write("field-test/round1/proj/a.md")
    world.write("field-test/round1/proj/FINDINGS.md", "# notes\n")
    assert world.run() == 0
    rows = world.rows()
    assert list(rows) == ["field-test/round1/proj/a.md"]
    row = rows["field-test/round1/proj/a.md"]
    assert (row["label"], row["group"], row["project"]) == (
        "honest",
        "proj/a.md",
        "proj",
    )


def test_one_project_lands_in_one_split_across_rounds(world, api):
    for name in ("a.md", "b.md", "c.md", "d.md"):
        world.write(f"field-test/round1/proj/{name}")
        world.write(f"field-test/round2/proj/{name}")
    world.run()
    assert len({row["split"] for row in world.rows().values()}) == 1


def test_the_split_of_a_project_never_changes(world, api):
    # SHA-256 parity of the project's name: "alpha" is even, "beta" odd.
    world.write("field-test/round1/alpha/a.md")
    world.write("field-test/round1/beta/a.md")
    world.run()
    rows = world.rows()
    assert rows["field-test/round1/alpha/a.md"]["split"] == "tune"
    assert rows["field-test/round1/beta/a.md"]["split"] == "heldout"


def test_screen_eval_folders_give_labels_and_filler_is_handed_on(world, api):
    world.write("screen-eval/public/p.md")
    world.write("screen-eval/review/r.md")
    world.write("screen-eval/rejected/football.md")
    world.write("screen-eval/rejected/filler.md")
    world.write("screen-eval/README.md", "# readme\n")
    world.run()
    labels = {case: row["label"] for case, row in world.rows().items()}
    assert labels == {
        "screen-eval/public/p.md": "honest",
        "screen-eval/review/r.md": "honest",
        "screen-eval/rejected/football.md": "junk",
        "screen-eval/rejected/filler.md": "hand_on",
    }


def test_a_junk_body_is_composed_under_each_template_of_its_split(world, api):
    world.write("first-screen-eval/tune/templates/one.txt", "ONE\n{{BODY}}\nend one\n")
    world.write("first-screen-eval/tune/templates/two.txt", "TWO\n{{BODY}}\nend two\n")
    world.write("first-screen-eval/heldout/templates/h.txt", "HELD\n{{BODY}}\n")
    world.write("first-screen-eval/tune/junk/list.txt", "milk\neggs\n")
    world.write("first-screen-eval/heldout/junk/ad.txt", "buy now\n")
    world.write("first-screen-eval/tune/honest/null.txt", "an honest entry\n")
    world.write("first-screen-eval/tune/hand-on/filler.txt", "filler\n")
    world.run()
    rows = world.rows()
    assert {case: (row["label"], row["split"]) for case, row in rows.items()} == {
        "first-screen-eval/tune/junk/list.txt@one": ("junk", "tune"),
        "first-screen-eval/tune/junk/list.txt@two": ("junk", "tune"),
        "first-screen-eval/heldout/junk/ad.txt@h": ("junk", "heldout"),
        "first-screen-eval/tune/honest/null.txt": ("honest", "tune"),
        "first-screen-eval/tune/hand-on/filler.txt": ("hand_on", "tune"),
    }
    assert "ONE\nmilk\neggs\nend one\n" in api.states()
    assert "TWO\nmilk\neggs\nend two\n" in api.states()
    assert "an honest entry\n" in api.states()
    bodies = {
        rows[f"first-screen-eval/tune/junk/list.txt@{t}"]["body"]
        for t in ("one", "two")
    }
    assert bodies == {"first-screen-eval/tune/junk/list.txt"}


def test_the_report_counts_junk_by_folder(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {f"first-screen-eval/tune/junk/j{n}.txt": f"p=0.95 {n}\n" for n in range(8)}
        | {"first-screen-eval/tune/junk-entries/caught.txt": "p=0.95 whole\n"}
        | {"first-screen-eval/tune/junk-entries/missed.txt": "p=0.20 whole\n"}
        | {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"},
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 0
    out = capsys.readouterr().out
    assert "junk bodies rejected: 9/10" in out
    assert "  in first-screen-eval/tune/junk: 8/8" in out
    assert "  in first-screen-eval/tune/junk-entries: 1/2" in out


def test_whole_junk_entries_and_honest_edge_cases_are_read(world, api):
    world.write("first-screen-eval/tune/templates/t.txt", "head\n{{BODY}}\n")
    world.write("first-screen-eval/tune/junk-entries/list.txt", "a labeled list\n")
    world.write("first-screen-eval/tune/honest-edge/sim.txt", "a simulation\n")
    world.run(split="tune")
    rows = world.rows()
    junk = rows["first-screen-eval/tune/junk-entries/list.txt"]
    assert (junk["label"], junk["body"]) == ("junk", junk["id"])
    assert rows["first-screen-eval/tune/honest-edge/sim.txt"]["label"] == "honest"
    assert sorted(api.states()) == ["a labeled list\n", "a simulation\n"]


def test_split_chooses_which_cases_are_sent(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "tune text\n")
    world.write("first-screen-eval/heldout/honest/b.txt", "held text\n")
    world.run(split="tune")
    assert api.states() == ["tune text\n"]


def test_the_request_names_the_model_the_text_and_the_questions(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "some text\n")
    world.run()
    assert api.requests == [
        {
            "model": MODEL,
            "state": "some text\n",
            "questions": {"junk": {"type": "noul", "instructions": "Is it junk?"}},
        }
    ]
    assert api.auth == [("/v1/systemone", f"Bearer {KEY}")]


def test_each_case_is_sent_once_per_draw_and_every_draw_is_kept(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    draws = iter([0.2, 0.9, 0.4])
    api.answer = lambda state: {"junk": next(draws)}
    world.run(draws="3")
    row = world.rows()["first-screen-eval/tune/honest/a.txt"]
    assert sorted(row["draws"]["junk"]) == [0.2, 0.4, 0.9]
    assert row["outcome"] == "scored"
    assert row["input_tokens"] == 100


def test_a_second_run_sends_nothing_for_finished_cases(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run(draws="2")
    assert len(api.requests) == 2
    world.write("first-screen-eval/tune/honest/b.txt", "y\n")
    world.run(draws="2")
    assert api.states() == ["x\n", "x\n", "y\n", "y\n"]
    assert len(world.rows()) == 2


def test_long_text_is_cut_at_a_line_end_within_32_kib(world, api):
    line = "é" * 499 + "\n"  # 999 bytes
    world.write("first-screen-eval/tune/honest/long.txt", line * 40)
    world.run()
    assert api.states() == [line * 32]
    assert world.rows()["first-screen-eval/tune/honest/long.txt"]["cut"] is True


def test_text_with_no_line_end_is_cut_at_a_whole_character(world, api):
    world.write("first-screen-eval/tune/honest/blob.txt", "a" + "é" * 20000)
    world.run()
    assert api.states() == ["a" + "é" * 16383]


def test_short_text_is_not_cut(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run()
    assert world.rows()["first-screen-eval/tune/honest/a.txt"]["cut"] is False


def test_an_empty_case_is_recorded_and_never_sent(world, api):
    world.write("first-screen-eval/tune/honest/empty.txt", "")
    assert world.run() == 0
    assert api.requests == []
    assert world.rows()["first-screen-eval/tune/honest/empty.txt"]["outcome"] == "empty"


def test_a_rate_limit_or_overload_is_retried(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    replies = iter(
        [
            (429, {"retry-after": "0"}, {"detail": "slow down"}),
            (529, {}, {"detail": "overloaded"}),
            {"junk": 0.1},
        ]
    )
    api.answer = lambda state: next(replies)
    assert world.run() == 0
    assert world.rows()["first-screen-eval/tune/honest/a.txt"]["draws"] == {
        "junk": [0.1]
    }


def test_a_rate_limit_that_never_lifts_stops_the_run(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    api.answer = lambda state: (429, {}, {"detail": "slow down"})
    assert world.run() == 1
    assert len(api.requests) == 5
    assert "429" in capsys.readouterr().err


def test_text_past_the_context_length_is_recorded_as_too_long(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    api.answer = lambda state: (
        400,
        {},
        {"detail": {"error_type": "max_tokens_exceeded"}},
    )
    assert world.run() == 0
    assert world.rows()["first-screen-eval/tune/honest/a.txt"]["outcome"] == "too_long"


def test_a_refused_request_stops_the_run_without_echoing_the_text(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "SECRET ENTRY TEXT\n")
    detail = [
        {
            "loc": ["body", "questions"],
            "msg": "bad question",
            "input": "SECRET ENTRY TEXT",
        }
    ]
    api.answer = lambda state: (422, {}, {"detail": detail})
    assert world.run() == 1
    err = capsys.readouterr().err
    assert "422" in err and "bad question" in err and "body.questions" in err
    assert "first-screen-eval/tune/honest/a.txt" in err
    assert "SECRET" not in err
    assert not (world.out / "scores.jsonl").exists() or world.rows() == {}


@pytest.mark.parametrize(
    "answer",
    [
        {},
        {"junk": 1.5},
        {"junk": "yes"},
        {"junk": True},
    ],
)
def test_an_answer_that_is_not_a_probability_stops_the_run(world, api, capsys, answer):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    api.answer = lambda state: answer
    assert world.run() == 1
    assert "first-screen-eval/tune/honest/a.txt" in capsys.readouterr().err
    assert not (world.out / "scores.jsonl").exists() or world.rows() == {}


def test_an_answer_from_another_model_stops_the_run(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    api.model = "jev-0.0.1"
    assert world.run() == 1
    assert "jev-0.0.1" in capsys.readouterr().err


def test_a_model_alias_is_refused_before_anything_is_sent(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.set_questions(
        {"junk": {"type": "noul", "instructions": "?"}}, model="jev-latest"
    )
    assert world.run() == 2
    assert api.requests == []
    assert "jev-latest" in capsys.readouterr().err


def test_a_server_error_stops_the_run_and_a_rerun_resumes(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "good\n")
    world.write("first-screen-eval/tune/honest/b.txt", "bad\n")
    api.answer = lambda state: {"junk": 0.1} if state == "good\n" else (500, {}, {})
    assert world.run("--workers", "1") == 1
    assert "500" in capsys.readouterr().err
    assert list(world.rows()) == ["first-screen-eval/tune/honest/a.txt"]
    api.requests.clear()
    api.answer = lambda state: {"junk": 0.2}
    assert world.run() == 0
    assert api.states() == ["bad\n"]


def test_an_unreachable_api_stops_the_run_with_one_line(
    world, api, monkeypatch, capsys
):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    monkeypatch.setenv("FIRST_SCREEN_API_URL", "http://127.0.0.1:1")
    assert world.run() == 1
    err = capsys.readouterr().err
    assert "Traceback" not in err and err.count("\n") == 1


def test_the_key_appears_in_no_output(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.write("first-screen-eval/tune/honest/b.txt", "y\n")
    api.answer = (
        lambda state: {"junk": 0.1} if state == "x\n" else (401, {}, {"detail": KEY})
    )
    world.run()
    captured = capsys.readouterr()
    assert KEY not in captured.out + captured.err
    assert KEY not in (world.out / "scores.jsonl").read_text()


def test_a_missing_key_is_a_usage_error(world, api, monkeypatch, capsys):
    monkeypatch.delenv("JEV_API_KEY")
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    assert world.run() == 2
    assert "JEV_API_KEY" in capsys.readouterr().err


def test_scores_from_other_questions_are_refused(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run()
    world.set_questions({"junk": {"type": "noul", "instructions": "A new wording?"}})
    assert world.run() == 2
    assert "other questions" in capsys.readouterr().err


def test_a_torn_last_line_is_skipped(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run()
    with (world.out / "scores.jsonl").open("a") as scores:
        scores.write('{"id": "first-screen-eval/tune/honest/b.txt", "dra')
    world.write("first-screen-eval/tune/honest/b.txt", "y\n")
    assert world.run() == 0
    assert set(world.rows()) == {
        "first-screen-eval/tune/honest/a.txt",
        "first-screen-eval/tune/honest/b.txt",
    }


def test_an_out_folder_git_would_track_is_refused(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.out = world.tmp / "results"
    assert world.run(split="tune") == 2
    assert api.requests == []
    assert "ignore" in capsys.readouterr().err


def test_an_out_folder_outside_any_work_tree_is_fine(world, api, tmp_path_factory):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.out = tmp_path_factory.mktemp("elsewhere") / "out"
    assert world.run(split="tune") == 0
    assert len(world.rows()) == 1


def test_entry_check_marks_what_the_server_would_refuse(world, api):
    world.write("first-screen-eval/tune/templates/t.txt", "head\n{{BODY}}\n")
    world.write("first-screen-eval/tune/junk/bad.txt", "REFUSEME\n")
    world.write("first-screen-eval/tune/junk/ok.txt", "fine\n")
    world.run("--entry-check", str(world.entry_check), split="tune")
    rows = world.rows()
    assert rows["first-screen-eval/tune/junk/bad.txt@t"]["refused"] is True
    assert rows["first-screen-eval/tune/junk/ok.txt@t"]["refused"] is False


def test_without_entry_check_refused_is_unknown(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run(split="tune")
    assert world.rows()["first-screen-eval/tune/honest/a.txt"]["refused"] is None


def test_a_heldout_run_needs_a_rule_and_entry_check(world, api):
    world.write("first-screen-eval/heldout/honest/a.txt", "x\n")
    assert world.run(split="heldout", guard=False) == 2
    assert world.run("--rule", str(world.rule), split="heldout", guard=False) == 2
    assert api.requests == []
    assert world.run(split="heldout") == 0
    assert world.rows()["first-screen-eval/heldout/honest/a.txt"]["rule_hash"]


@pytest.mark.parametrize("changed", ["questions", "rule", "untracked rule"])
def test_a_heldout_run_refuses_questions_or_a_rule_that_differ_from_head(
    world, api, capsys, changed
):
    world.write("first-screen-eval/heldout/honest/a.txt", "x\n")
    rule = world.rule
    if changed == "questions":
        world.set_questions(
            {"junk": {"type": "noul", "instructions": "Reworded?"}}, commit=False
        )
    elif changed == "rule":
        rule.write_text(json.dumps({"reject_if_any": []}))
    else:
        rule = world.tmp / "other-rule.json"
        rule.write_text(json.dumps({"reject_if_any": []}))
    argv = ["--rule", str(rule), "--entry-check", str(world.entry_check)]
    assert world.run(*argv, split="heldout", guard=False) == 2
    assert api.requests == []
    assert "commit" in capsys.readouterr().err


def test_all_includes_the_heldout_split_only_with_its_guard(world, api):
    world.write("first-screen-eval/heldout/honest/a.txt", "x\n")
    assert world.run(split="all", guard=False) == 2
    assert api.requests == []


def scored(world, api, cases, split="tune", draws="1"):
    """Runs `cases`, a map of path to text, with the fake answering each text's
    `p=` value, or the next of several `p=` values on each draw."""
    for path, text in cases.items():
        world.write(path, text)
    seen = {}

    def answer(state):
        values = [float(part[:4]) for part in state.split("p=")[1:]]
        nth = seen.get(state, 0)
        seen[state] = nth + 1
        return {"junk": values[nth % len(values)]}

    api.answer = answer
    assert world.run(split=split, draws=draws) == 0


TEMPLATE = {"first-screen-eval/tune/templates/t.txt": "head\n{{BODY}}\n"}


def test_the_sweep_counts_what_each_threshold_would_reject(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {
            "first-screen-eval/tune/junk/a.txt": "p=0.95\n",
            "first-screen-eval/tune/junk/b.txt": "p=0.60\n",
            "first-screen-eval/tune/honest/h1.txt": "p=0.70\n",
            "first-screen-eval/tune/honest/h2.txt": "p=0.10\n",
            "first-screen-eval/tune/honest/h3.txt": "p=0.02\n",
        },
    )
    capsys.readouterr()
    assert world.report() == 0
    out = capsys.readouterr().out
    assert "  at_least 0.90  junk 1/2  honest 0/3" in out
    assert "  at_least 0.60  junk 2/2  honest 1/3" in out
    assert "  at_most 0.10  junk 0/2  honest 2/3" in out
    assert "  at_most 0.05  junk 0/2  honest 1/3" in out


def rule(world, *conditions):
    world.rule.write_text(json.dumps({"reject_if_any": list(conditions)}))
    world.commit()
    return ["--rule", str(world.rule)]


def test_a_rule_that_rejects_the_junk_and_no_honest_entry_passes(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {f"first-screen-eval/tune/junk/j{n}.txt": f"p=0.95 {n}\n" for n in range(4)}
        | {"first-screen-eval/tune/junk/missed.txt": "p=0.30\n"}
        | {
            f"first-screen-eval/tune/honest/h{n}.txt": f"p=0.10 {n}\n"
            for n in range(100)
        },
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 0
    out = capsys.readouterr().out
    assert "junk bodies rejected: 4/5" in out
    assert "first-screen-eval/tune/junk/missed.txt" in out
    assert "honest entries rejected: 0/100" in out
    assert "at most 3.0% of honest entries" in out
    assert out.rstrip().endswith("PASS")


def test_one_honest_entry_rejected_fails_and_is_named(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"first-screen-eval/tune/honest/unlucky.txt": "p=0.92\n"}
        | {
            f"first-screen-eval/tune/honest/h{n}.txt": f"p=0.10 {n}\n"
            for n in range(99)
        },
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 1
    out = capsys.readouterr().out
    assert "honest entries rejected: 1/100" in out
    assert "first-screen-eval/tune/honest/unlucky.txt" in out
    # Wilson's upper 95% bound for 1 of 100.
    assert "at most 5.4% of honest entries" in out
    assert out.rstrip().endswith("FAIL")


def test_too_little_junk_rejected_fails(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {f"first-screen-eval/tune/junk/j{n}.txt": f"p=0.95 {n}\n" for n in range(3)}
        | {f"first-screen-eval/tune/junk/m{n}.txt": f"p=0.20 {n}\n" for n in range(2)}
        | {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"},
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 1
    out = capsys.readouterr().out
    assert "junk bodies rejected: 3/5" in out
    assert out.rstrip().endswith("FAIL")


def test_a_junk_body_counts_only_when_every_template_is_rejected(world, api, capsys):
    scored(
        world,
        api,
        {
            "first-screen-eval/tune/templates/plain.txt": "p=0.95\n{{BODY}}\n",
            "first-screen-eval/tune/templates/dressed.txt": "p=0.40\n{{BODY}}\n",
            "first-screen-eval/tune/junk/j.txt": "list\n",
            "first-screen-eval/tune/honest/h.txt": "p=0.10\n",
        },
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 1
    out = capsys.readouterr().out
    assert "junk bodies rejected: 0/1" in out
    assert "under plain: 1/1" in out
    assert "under dressed: 0/1" in out


def test_any_draw_that_meets_a_condition_rejects(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"first-screen-eval/tune/honest/wobbly.txt": "p=0.85 p=0.91 p=0.80\n"},
        draws="3",
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 1
    out = capsys.readouterr().out
    assert "honest entries rejected: 1/1" in out
    assert "widest spread between draws: junk 0.11" in out


def test_versions_of_one_entry_count_as_one_honest_entry(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"field-test/round1/proj/a.md": "---\np=0.10 v1\n"}
        | {"field-test/round2/proj/a.md": "---\np=0.10 v2\n"}
        | {"field-test/round1/proj/b.md": "---\np=0.10 b\n"},
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 0
    out = capsys.readouterr().out
    assert "honest entries rejected: 0/2 (1 project)" in out
    assert "at most 100.0% of honest entries" in out


def test_at_most_is_skipped_for_a_case_that_was_cut(world, api, capsys):
    long_honest = "p=0.02\n" + "filler line\n" * 4000
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.01\n"}
        | {"first-screen-eval/tune/honest/long.txt": long_honest},
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_most": 0.05})) == 0
    out = capsys.readouterr().out
    assert "junk bodies rejected: 1/1" in out
    assert "honest entries rejected: 0/1" in out


def test_hand_on_cases_are_reported_and_scored_as_neither(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"}
        | {"first-screen-eval/tune/hand-on/filler.txt": "p=0.99\n"},
    )
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 0
    out = capsys.readouterr().out
    assert "hand_on cases rejected: 1/1" in out
    assert "honest entries rejected: 0/1" in out


def test_cases_the_server_would_refuse_are_left_out_of_the_counts(world, api, capsys):
    for path, text in (
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"first-screen-eval/tune/junk/never.txt": "p=0.10 REFUSEME\n"}
        | {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"}
        | {"first-screen-eval/tune/honest/old.txt": "p=0.99 REFUSEME\n"}
    ).items():
        world.write(path, text)
    api.answer = lambda state: {"junk": float(state.split("p=")[1][:4])}
    world.run("--entry-check", str(world.entry_check), split="tune")
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 0
    out = capsys.readouterr().out
    assert "junk bodies rejected: 1/1" in out
    assert "honest entries rejected: 0/1" in out
    assert "2 cases the server would refuse at submission are left out" in out


def test_the_report_says_when_entry_check_was_not_run(world, api, capsys):
    scored(world, api, {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"})
    capsys.readouterr()
    world.report()
    assert "entry-check wasn't run" in capsys.readouterr().out


def test_text_too_long_for_the_model_counts_as_handed_on(world, api, capsys):
    world.write("first-screen-eval/tune/templates/t.txt", "head\n{{BODY}}\n")
    world.write("first-screen-eval/tune/junk/huge.txt", "huge\n")
    world.write("first-screen-eval/tune/junk/j.txt", "p=0.95\n")
    world.write("first-screen-eval/tune/honest/h.txt", "p=0.10\n")
    too_long = (400, {}, {"detail": {"error_type": "max_tokens_exceeded"}})
    api.answer = lambda state: (
        too_long if "huge" in state else {"junk": float(state.split("p=")[1][:4])}
    )
    world.run(split="tune")
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 1
    out = capsys.readouterr().out
    assert "junk bodies rejected: 1/2" in out
    assert "1 case too long for the model" in out


def test_the_cost_of_one_screening_is_printed(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"},
    )
    capsys.readouterr()
    world.report(*rule(world, {"question": "junk", "at_least": 0.9}))
    # The fake bills 100 input tokens a call, at $0.042 a million.
    assert "100 input tokens a call on average, $0.0000042" in capsys.readouterr().out


def test_a_rule_naming_an_unknown_question_is_a_usage_error(world, api, capsys):
    scored(world, api, {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"})
    assert world.report(*rule(world, {"question": "nope", "at_least": 0.9})) == 2
    assert "nope" in capsys.readouterr().err


HELDOUT = {"first-screen-eval/heldout/templates/t.txt": "head\n{{BODY}}\n"}


def test_the_heldout_report_needs_the_rule_the_run_was_given(world, api, capsys):
    scored(
        world,
        api,
        HELDOUT | {"first-screen-eval/heldout/honest/h.txt": "p=0.10\n"},
        split="heldout",
    )
    assert world.report("--split", "heldout") == 2
    other = world.tmp / "other.json"
    other.write_text(json.dumps({"reject_if_any": []}))
    assert world.report("--split", "heldout", "--rule", str(other)) == 2
    assert "rule" in capsys.readouterr().err


def test_a_heldout_set_with_under_20_junk_bodies_is_insufficient(world, api, capsys):
    scored(
        world,
        api,
        HELDOUT
        | {
            f"first-screen-eval/heldout/junk/j{n}.txt": f"p=0.95 {n}\n"
            for n in range(19)
        }
        | {"first-screen-eval/heldout/honest/h.txt": "p=0.10\n"},
        split="heldout",
    )
    capsys.readouterr()
    assert world.report("--split", "heldout", "--rule", str(world.rule)) == 1
    assert (
        capsys.readouterr().out.rstrip().endswith("INSUFFICIENT: under 20 junk bodies")
    )


def test_a_heldout_set_with_20_junk_bodies_can_pass(world, api, capsys):
    scored(
        world,
        api,
        HELDOUT
        | {
            f"first-screen-eval/heldout/junk/j{n}.txt": f"p=0.95 {n}\n"
            for n in range(20)
        }
        | {"first-screen-eval/heldout/honest/h.txt": "p=0.10\n"},
        split="heldout",
    )
    capsys.readouterr()
    assert world.report("--split", "heldout", "--rule", str(world.rule)) == 0
    assert capsys.readouterr().out.rstrip().endswith("PASS")


def test_the_report_refuses_scores_from_mixed_questions(world, api, capsys):
    scored(world, api, {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"})
    scores = world.out / "scores.jsonl"
    row = json.loads(scores.read_text())
    row["questions_hash"] = "another"
    with scores.open("a") as file:
        file.write(json.dumps(row) + "\n")
    assert world.report() == 2
    assert "mix" in capsys.readouterr().err


@pytest.mark.parametrize(
    "reply",
    [
        (200, {}, {"model": MODEL, "answers": {"junk": None}, "usage": {}}),
        (200, {}, {"model": MODEL, "answers": None, "usage": {}}),
        (200, {}, ["not", "an", "object"]),
        (200, {}, {"model": MODEL, "answers": {"junk": {"noul": 0.5}}, "usage": None}),
    ],
)
def test_a_reply_of_another_shape_stops_the_run_naming_the_case(
    world, api, capsys, reply
):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    api.answer = lambda state: reply
    assert world.run(split="tune") == 1
    err = capsys.readouterr().err
    assert "first-screen-eval/tune/honest/a.txt" in err
    assert "Traceback" not in err and err.count("\n") == 1


def test_a_retry_after_that_is_a_date_falls_back_to_the_backoff(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    replies = iter(
        [(429, {"retry-after": "Wed, 21 Oct 2026 07:28:00 GMT"}, {}), {"junk": 0.1}]
    )
    api.answer = lambda state: next(replies)
    assert world.run(split="tune") == 0


def test_two_cases_with_the_same_text_each_get_a_row(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "same\n")
    world.run(split="tune")
    world.write("first-screen-eval/tune/honest/b.txt", "same\n")
    world.write("first-screen-eval/tune/honest/e1.txt", "")
    world.write("first-screen-eval/tune/honest/e2.txt", "")
    world.run(split="tune")
    assert len(world.rows()) == 4
    assert len(api.requests) == 2


def test_a_rerun_with_more_draws_replaces_the_row(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run(split="tune", draws="1")
    world.run(split="tune", draws="3")
    lines = (world.out / "scores.jsonl").read_text().splitlines()
    assert len(lines) == 1
    assert len(json.loads(lines[0])["draws"]["junk"]) == 3


def test_a_rerun_with_entry_check_fills_in_what_was_unknown(world, api):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.run(split="tune")
    world.run("--entry-check", str(world.entry_check), split="tune")
    lines = (world.out / "scores.jsonl").read_text().splitlines()
    assert [json.loads(line)["refused"] for line in lines] == [False]


def test_an_entry_check_that_fails_to_run_stops_the_run(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.entry_check.write_text("#!/bin/sh\nexit 2\n")
    assert world.run("--entry-check", str(world.entry_check), split="tune") == 1
    assert "exited 2" in capsys.readouterr().err
    assert api.requests == []


@pytest.mark.parametrize(
    "argv",
    [
        ["--entry-check", "/nonexistent/entry-check"],
        ["--draws", "0"],
        ["--rule", "/nonexistent/rule.json"],
    ],
)
def test_arguments_that_cannot_work_are_usage_errors(world, api, capsys, argv):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    assert (
        world.run(*argv, split="tune", draws=argv[1] if argv[0] == "--draws" else "1")
        == 2
    )
    err = capsys.readouterr().err
    assert "Traceback" not in err and err.count("\n") == 1
    assert api.requests == []


def test_questions_that_cannot_be_read_are_a_usage_error(world, api, capsys):
    world.write("first-screen-eval/tune/honest/a.txt", "x\n")
    world.questions.write_text("{not json")
    assert world.run(split="tune") == 2
    assert "questions.json" in capsys.readouterr().err


def test_a_report_with_no_junk_cannot_pass(world, api, capsys):
    scored(world, api, {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"})
    capsys.readouterr()
    assert world.report(*rule(world, {"question": "junk", "at_least": 0.9})) == 1
    assert capsys.readouterr().out.rstrip().endswith("INSUFFICIENT: no junk bodies")


def test_the_report_totals_what_the_run_was_billed(world, api, capsys):
    scored(
        world,
        api,
        TEMPLATE
        | {"first-screen-eval/tune/junk/j.txt": "p=0.95\n"}
        | {"first-screen-eval/tune/honest/h.txt": "p=0.10\n"},
        draws="3",
    )
    capsys.readouterr()
    world.report()
    # 2 cases, 3 draws each, 100 tokens a call.
    assert "600 input tokens in all, $0.0000252" in capsys.readouterr().out


def test_help_runs_from_the_command_line_and_names_every_option():
    done = subprocess.run(
        [str(REPO / "ops/first-screen-bakeoff"), "-h"],
        capture_output=True,
        text=True,
        check=True,
    )
    for option in ("--questions", "--split", "--draws", "--workers", "--out", "--rule"):
        assert option in done.stdout
    for option in ("--entry-check", "--field-test", "--cases", "--screen-eval"):
        assert option in done.stdout
