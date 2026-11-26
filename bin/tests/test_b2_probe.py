"""Tests for ops/b2-probe (#156), which the owner runs against throwaway B2 buckets.

B2 is bin/tests/fake_b2.py, a local server with a clock of its own, and Litestream
is bin/tests/fake_litestream.py inside a fake release archive served over file://,
so the pinned download path runs too. The probe runs in-process, its clock tied to
the fake's. Nothing here reaches the network.
"""

import hashlib
import importlib.machinery
import importlib.util
import io
import json
import subprocess
import sys
import tarfile
import time
from pathlib import Path

import pytest
from fake_b2 import MASTER_ID, MASTER_KEY, FakeB2

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "ops/b2-probe"
FAKE_LITESTREAM = Path(__file__).resolve().parent / "fake_litestream.py"


def load():
    loader = importlib.machinery.SourceFileLoader("b2_probe", str(SCRIPT))
    spec = importlib.util.spec_from_loader("b2_probe", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


PROBE = load()


def fake_archive(path: Path) -> str:
    """A Litestream release archive whose binary runs fake_litestream.py; returns
    its SHA-256."""
    script = (
        f'#!/bin/sh\nexec "{sys.executable}" -I "{FAKE_LITESTREAM}" "$@"\n'.encode()
    )
    with tarfile.open(path, "w:gz") as tar:
        for name, data, mode in [
            ("README.md", b"readme\n", 0o644),
            ("litestream", script, 0o755),
        ]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = mode
            tar.addfile(info, io.BytesIO(data))
    return hashlib.sha256(path.read_bytes()).hexdigest()


@pytest.fixture
def b2(monkeypatch, tmp_path):
    fake = FakeB2()
    archive = tmp_path / "litestream.tar.gz"
    monkeypatch.setenv("FILEDRAWER_B2_API_URL", fake.url)
    monkeypatch.setenv("FILEDRAWER_LITESTREAM_URL", archive.as_uri())
    monkeypatch.setenv("FILEDRAWER_LITESTREAM_SHA256", fake_archive(archive))
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path / "cache"))
    monkeypatch.setenv("B2_APPLICATION_KEY_ID", MASTER_ID)
    monkeypatch.setenv("B2_APPLICATION_KEY", MASTER_KEY)
    # An owner's own AWS setting, which must not reach Litestream (fake_litestream exits 9).
    monkeypatch.setenv("AWS_PROFILE", "owners-own")
    monkeypatch.setattr(PROBE, "now", fake.now)
    monkeypatch.setattr(PROBE, "pause", lambda seconds: fake.advance(seconds / 86400))
    # Big enough to go up in parts at the default part size, small enough to be quick.
    monkeypatch.setattr(PROBE, "BIG_DB_BYTES", 6 * 1024 * 1024)
    yield fake
    fake.stop()


@pytest.fixture
def state(tmp_path) -> Path:
    return tmp_path / "state" / "b2-probe.json"


def probe(capsys, *argv: str) -> tuple[int, str]:
    status = PROBE.main(list(argv))
    out, err = capsys.readouterr()
    return status, out + err


def lines(output: str, tag: str) -> list[str]:
    return [line for line in output.splitlines() if line.startswith(tag)]


def assert_no_secret(b2: FakeB2, *texts: str) -> None:
    for secret in [k["secret"] for k in b2.keys.values()] + [MASTER_KEY]:
        for text in texts:
            assert secret not in text


# --- SigV4 ----------------------------------------------------------------------

# AWS's worked examples for S3's Signature Version 4, "Signature Calculations for
# the Authorization Header: Transferring Payload in a Single Chunk".
AWS_KEY_ID = "AKIAIOSFODNN7EXAMPLE"
AWS_SECRET = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
AWS_WHEN = time.strptime("20130524T000000Z", "%Y%m%dT%H%M%SZ")


@pytest.mark.parametrize(
    "url, headers, signature",
    [
        (
            "https://examplebucket.s3.amazonaws.com/test.txt",
            {"Range": "bytes=0-9"},
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41",
        ),
        (
            "https://examplebucket.s3.amazonaws.com/?lifecycle",
            {},
            "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543",
        ),
    ],
)
def test_sign_v4_matches_awss_worked_examples(url, headers, signature):
    signed = PROBE.sign_v4(
        "GET", url, headers, b"", AWS_KEY_ID, AWS_SECRET, "us-east-1", AWS_WHEN
    )
    assert signed["Authorization"].endswith(f"Signature={signature}")
    assert (
        f"Credential={AWS_KEY_ID}/20130524/us-east-1/s3/aws4_request"
        in signed["Authorization"]
    )


def test_region_comes_from_b2s_s3_host():
    assert PROBE.s3_region("https://s3.us-west-004.backblazeb2.com") == "us-west-004"
    assert PROBE.s3_region("http://127.0.0.1:9") == "us-east-1"


# --- run ------------------------------------------------------------------------


def test_run_passes_each_check_and_leaves_only_the_buckets(b2, state, capsys):
    status, out = probe(capsys, "run", "--state", str(state))

    assert status == 0, out
    assert lines(out, "FAIL") == []
    passed = "\n".join(lines(out, "PASS"))
    for check in [
        "S3 delete by name (plain bucket)",
        "S3 delete by name (lock bucket)",
        "b2_hide_file",
        "b2_delete_file_version refused",
        "S3 delete with a version id refused",
        "the master key deletes that version",
        "the lock canary is locked for 3 days",
        "Litestream replicates into the lock bucket",
        "Litestream, part-size 128MiB",
        "Litestream's retention pass in the lock bucket",
        "Litestream restore",
        "Litestream after a rollback and reset",
    ]:
        assert f"PASS   {check}" in passed, out
    [answer] = lines(out, "ANSWER")
    assert (
        "default part-size" in answer
        and "refused" in answer
        and "Content-MD5" in answer
    )

    # The keys are gone; the buckets wait for recheck, with the settings #156 names.
    assert b2.restricted_keys() == []
    buckets = {b["name"].rsplit("-", 1)[1]: b for b in b2.buckets.values()}
    assert set(buckets) == {"lock", "plain"}
    assert buckets["lock"]["locked"] and buckets["lock"]["retention_days"] == 1
    assert not buckets["plain"]["locked"]
    for bucket in buckets.values():
        assert bucket["lifecycle"] == [
            {"fileNamePrefix": "", "daysFromHidingToDeleting": 1}
        ]
    assert "Left in B2" in out
    saved = state.read_text()
    assert set(json.loads(saved)["canaries"]) == {"lock", "plain"}
    assert_no_secret(b2, out, saved)


def test_litestream_writes_with_the_lock_buckets_key_only(b2, state, capsys):
    made = []
    real_create = b2.native

    def note_keys(name, key_id, body):
        status, out = real_create(name, key_id, body)
        if name == "b2_create_key":
            made.append((out["applicationKeyId"], body["bucketIds"][0]))
        return status, out

    b2.native = note_keys
    status, out = probe(capsys, "run", "--state", str(state))

    assert status == 0, out
    lock_id = next(b["id"] for b in b2.buckets.values() if b["locked"])
    [lock_key] = [key for key, bucket in made if bucket == lock_id]
    writes = [r for r in b2.s3_requests if r[1] == "PUT"]
    assert writes and {key for key, _, _ in writes} == {lock_key}


def test_run_answers_when_b2_takes_a_multipart_upload_without_a_checksum(
    b2, state, capsys
):
    b2.refuse_multipart_in_lock = False

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == 0, out
    [answer] = lines(out, "ANSWER")
    assert "accepted" in answer


def test_run_fails_a_key_that_can_delete_versions(b2, state, capsys):
    b2.restricted_can_delete = True

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == PROBE.EXIT_FAIL
    failed = "\n".join(lines(out, "FAIL"))
    assert "b2_delete_file_version refused" in failed
    assert "S3 delete with a version id refused" in failed
    assert b2.restricted_keys() == []


def test_run_asks_for_the_master_key_when_the_environment_lacks_it(
    b2, state, capsys, monkeypatch
):
    monkeypatch.delenv("B2_APPLICATION_KEY")
    monkeypatch.setattr("builtins.input", lambda prompt: MASTER_ID)
    monkeypatch.setattr(PROBE.getpass, "getpass", lambda prompt: MASTER_KEY)

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == 0, out
    assert_no_secret(b2, out)


def test_a_refused_master_key_stops_before_making_anything(
    b2, state, capsys, monkeypatch
):
    monkeypatch.setenv("B2_APPLICATION_KEY", "wrong-secret-value")

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == PROBE.EXIT_ERROR
    assert "master application key" in out
    assert "wrong-secret-value" not in out
    assert b2.buckets == {}


def test_run_refuses_while_an_earlier_probe_is_unfinished(b2, state, capsys):
    probe(capsys, "run", "--state", str(state))
    buckets = dict(b2.buckets)

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == PROBE.EXIT_ERROR
    assert "recheck or cleanup" in out
    assert b2.buckets.keys() == buckets.keys()


def test_a_failed_step_still_deletes_the_keys(b2, state, capsys, monkeypatch):
    monkeypatch.setenv("FILEDRAWER_LITESTREAM_SHA256", "0" * 64)

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == PROBE.EXIT_FAIL
    assert "pinned" in "\n".join(lines(out, "FAIL"))
    assert b2.restricted_keys() == []
    assert not (Path(PROBE.os.environ["XDG_CACHE_HOME"]) / "filedrawer").exists()


# --- recheck and cleanup --------------------------------------------------------


def test_recheck_waits_while_the_lifecycle_hasnt_run(b2, state, capsys):
    probe(capsys, "run", "--state", str(state))
    b2.advance(1.5)

    status, out = probe(capsys, "recheck", "--state", str(state))

    assert status == PROBE.EXIT_WAIT
    assert "plain bucket's hidden canary is still there" in lines(out, "WAIT")[0]
    assert len(b2.buckets) == 2 and state.exists()


def daily_lifecycle(b2: FakeB2, days: int) -> None:
    for _ in range(days):
        b2.advance(1)
        b2.run_lifecycle()


def test_recheck_answers_yes_and_removes_everything(b2, state, capsys):
    probe(capsys, "run", "--state", str(state))
    daily_lifecycle(b2, 5)

    status, out = probe(capsys, "recheck", "--state", str(state))

    assert status == 0, out
    [answer] = lines(out, "ANSWER")
    assert "deletes a hidden version it skipped for its lock" in answer
    [note] = [
        line for line in out.splitlines() if "Litestream's hidden versions" in line
    ]
    gone, of = note.split(": ")[1].removesuffix(" gone").split(" of ")
    assert gone == of != "0"
    assert b2.buckets == {}
    assert not state.exists()
    assert "Nothing left to clean up" in out


def test_the_lock_canary_is_still_locked_when_the_lifecycle_first_may_delete_it(
    b2, state, capsys
):
    """Day 1's pass skips it for its lock, so the answer is about what B2 does later."""
    probe(capsys, "run", "--state", str(state))
    daily_lifecycle(b2, 2)

    lock = next(b for b in b2.buckets.values() if b["locked"])
    assert any(
        v["fileName"] == PROBE.CANARY and v["action"] == "upload"
        for v in lock["versions"]
    )
    plain = next(b for b in b2.buckets.values() if not b["locked"])
    assert not any(
        v["fileName"] == PROBE.CANARY and v["action"] == "upload"
        for v in plain["versions"]
    )


def test_recheck_answers_no_when_the_locked_version_outlives_its_lock(
    b2, state, capsys
):
    b2.lifecycle_retries_after_lock = False
    probe(capsys, "run", "--state", str(state))
    daily_lifecycle(b2, 6)

    status, out = probe(capsys, "recheck", "--state", str(state))

    assert status == 0, out
    [answer] = lines(out, "ANSWER")
    assert "does NOT delete" in answer
    assert b2.buckets == {}


def test_recheck_waits_for_the_lock_to_lapse_before_answering_no(b2, state, capsys):
    b2.lifecycle_retries_after_lock = False
    probe(capsys, "run", "--state", str(state))
    daily_lifecycle(b2, 4)

    status, out = probe(capsys, "recheck", "--state", str(state))

    assert status == PROBE.EXIT_WAIT
    assert "the plain bucket's canary is gone; the lock lapsed" in lines(out, "WAIT")[0]
    assert len(b2.buckets) == 2


def test_recheck_fails_a_lifecycle_that_deletes_a_locked_version(b2, state, capsys):
    b2.lifecycle_ignores_lock = True
    probe(capsys, "run", "--state", str(state))
    daily_lifecycle(b2, 2)

    status, out = probe(capsys, "recheck", "--state", str(state))

    assert status == PROBE.EXIT_FAIL
    assert "went before its lock lapsed" in lines(out, "FAIL")[0]


def test_recheck_keep_leaves_the_buckets(b2, state, capsys):
    probe(capsys, "run", "--state", str(state))
    daily_lifecycle(b2, 5)

    status, out = probe(capsys, "recheck", "--keep", "--state", str(state))

    assert status == 0
    assert len(b2.buckets) == 2 and state.exists()


def test_cleanup_before_the_lock_lapses_keeps_the_lock_bucket(b2, state, capsys):
    probe(capsys, "run", "--state", str(state))

    status, out = probe(capsys, "cleanup", "--state", str(state))

    assert status == 0, out
    [left] = b2.buckets.values()
    assert left["locked"]
    assert f"bucket {left['name']} kept" in out and "locks lapse" in out
    assert state.exists()

    b2.advance(3.1)
    status, out = probe(capsys, "cleanup", "--state", str(state))

    assert status == 0, out
    assert b2.buckets == {} and not state.exists()


def test_cleanup_refuses_a_bucket_outside_the_probes_names(b2, state, capsys):
    probe(capsys, "run", "--state", str(state))
    b2.advance(3.1)
    data = json.loads(state.read_text())
    plain = data["buckets"]["plain"]
    b2.buckets[plain["id"]]["name"] = plain["name"] = "owners-real-backups"
    state.write_text(json.dumps(data))

    status, out = probe(capsys, "cleanup", "--state", str(state))

    assert status == PROBE.EXIT_FAIL
    assert "cleanup refused bucket owners-real-backups" in lines(out, "FAIL")[0]
    assert b2.buckets[plain["id"]]["versions"], "nothing in it was deleted"
    assert [b["name"] for b in b2.buckets.values()] == ["owners-real-backups"]


def test_a_signal_mid_run_still_deletes_the_keys(b2, state, capsys, monkeypatch):
    def killed(*args):
        PROBE.interrupted(PROBE.signal.SIGTERM, None)

    monkeypatch.setattr(PROBE, "check_litestream", killed)

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == PROBE.EXIT_FAIL
    assert "Interrupted: SIGTERM" in lines(out, "FAIL")[0]
    assert b2.restricted_keys() == []
    assert "Left in B2" in out


def test_an_unexpected_error_is_blanked_and_still_deletes_the_keys(
    b2, state, capsys, monkeypatch
):
    def broken(*args):
        raise RuntimeError(f"boom with {MASTER_KEY}")

    monkeypatch.setattr(PROBE, "plant_canaries", broken)

    status, out = probe(capsys, "run", "--state", str(state))

    assert status == PROBE.EXIT_FAIL
    assert "RuntimeError: boom with [secret]" in lines(out, "FAIL")[0]
    assert b2.restricted_keys() == []
    assert_no_secret(b2, out)


def test_an_unexpected_error_in_cleanup_still_lists_the_leftovers(
    b2, state, capsys, monkeypatch
):
    probe(capsys, "run", "--state", str(state))

    def broken(*args):
        raise RuntimeError(f"boom with {MASTER_KEY}")

    monkeypatch.setattr(PROBE, "cleanup", broken)
    status, out = probe(capsys, "cleanup", "--state", str(state))

    assert status == PROBE.EXIT_ERROR
    assert "boom with [secret]" in out and "Left in B2" in out
    assert_no_secret(b2, out)


def test_recheck_without_a_probe_says_so(b2, state, capsys):
    status, out = probe(capsys, "recheck", "--state", str(state))

    assert status == PROBE.EXIT_ERROR
    assert "no probe state" in out


# --- Litestream's pinned download -----------------------------------------------


def test_litestream_is_cached_after_one_checked_download(b2, tmp_path):
    first = PROBE.litestream()
    (tmp_path / "litestream.tar.gz").unlink()

    assert PROBE.litestream() == first
    assert first.read_bytes().startswith(b"#!/bin/sh")


def test_the_pins_name_every_target():
    assert set(PROBE.TARGETS.values()) == set(PROBE.PINS)


def test_help_runs():
    done = subprocess.run(
        [str(SCRIPT), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert "recheck" in done.stdout
