"""Tests for the server's memory check (#64; design.md, Hosting and operations):
ops/remote/filedrawer-memory-check, which the timer runs, against a fake
/proc/meminfo and /proc/vmstat under a temp root, a temp STATE_DIRECTORY and a
fake curl first on PATH; and
ops/remote/memory.sh, which stores its ping URL and installs its unit and timer,
against a temp root with a fake systemctl.
"""

import os
import stat
import subprocess
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[2]
CHECK = REPO / "ops/remote/filedrawer-memory-check"
INSTALL = REPO / "ops/remote/memory.sh"
PING = "https://hc-ping.com/aaaaaaaa-0000-4000-8000-000000000064"
OTHER_PING = "https://hc-ping.com/bbbbbbbb-0000-4000-8000-000000000064"
# A 4 GiB server, as /proc/meminfo gives it in kB, with a 1 GiB swapfile.
TOTAL = 4 * 1024 * 1024
SWAP = 1024 * 1024


def executable(path: Path, text: str) -> Path:
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def meminfo(
    available: int, swap_free: int = SWAP, swap_total: int = SWAP, total: int = TOTAL
) -> str:
    """A /proc/meminfo as Debian's kernel writes it, cut to a few fields, with
    the page cache large, as a server's is: it counts as available."""
    return (
        f"MemTotal:       {total:>8} kB\n"
        f"MemFree:        {available // 4:>8} kB\n"
        f"MemAvailable:   {available:>8} kB\n"
        f"Buffers:        {1024:>8} kB\n"
        f"Cached:         {available // 2:>8} kB\n"
        f"SwapCached:     {0:>8} kB\n"
        f"SwapTotal:      {swap_total:>8} kB\n"
        f"SwapFree:       {swap_free:>8} kB\n"
    )


def vmstat(pswpout: int) -> str:
    """A /proc/vmstat, cut to the lines around the one the check reads."""
    return f"nr_free_pages 12345\npswpin 7\npswpout {pswpout}\npgfault 99\n"


@pytest.fixture
def check(tmp_path):
    root = tmp_path / "root"
    (root / "proc").mkdir(parents=True)
    state = tmp_path / "state"
    state.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    pings = tmp_path / "pings"
    pings.mkdir()
    # curl notes its arguments and its stdin, one pair of files a call, and exits
    # with the status in curl-status, if any.
    executable(
        fakes / "curl",
        f"""#!/bin/sh
n=$(find {pings} -name '*.args' | wc -l)
printf '%s\\n' "$@" > {pings}/$n.args
cat > {pings}/$n.stdin
[ -f {tmp_path}/curl-status ] && exit "$(cat {tmp_path}/curl-status)"
exit 0
""",
    )
    env = {
        "PATH": f"{fakes}:{os.environ['PATH']}",
        "FILEDRAWER_ROOT": str(root),
        "MEMORY_PING": PING,
        "STATE_DIRECTORY": str(state),
    }

    def run(
        info: str | None, pswpout: int | None = 0, **extra
    ) -> subprocess.CompletedProcess:
        if info is not None:
            (root / "proc/meminfo").write_text(info)
        if pswpout is not None:
            (root / "proc/vmstat").write_text(vmstat(pswpout))
        else:
            (root / "proc/vmstat").unlink(missing_ok=True)
        return subprocess.run(
            [str(CHECK)],
            env={**env, **extra},
            capture_output=True,
            text=True,
            check=False,
        )

    def sent() -> list[SimpleNamespace]:
        """Each curl call: the URL its config named, and its arguments."""
        calls = []
        for n in range(len(list(pings.glob("*.args")))):
            config = (pings / f"{n}.stdin").read_text()
            args = (pings / f"{n}.args").read_text().splitlines()
            url = config.split('url = "', 1)[1].split('"', 1)[0]
            # The body may run over several lines; the check's last two arguments
            # are -K -.
            start = args.index("--data-raw") + 1
            body = "\n".join(args[start:-2])
            calls.append(SimpleNamespace(url=url, args=args, body=body))
        return calls

    return SimpleNamespace(
        run=run, sent=sent, status=tmp_path / "curl-status", state=state / "pswpout"
    )


LOW = 3 * 1024 * 1024  # MemAvailable on a 4 GiB server with 1 GiB in use


def test_ram_under_half_and_no_swap_pings_success_with_the_figures(check):
    # 1 GiB of 4 in use.
    result = check.run(meminfo(available=3 * 1024 * 1024))
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING
    assert "RAM: 1024 of 4096 MiB in use (25%)" in ping.body
    assert "swap: 0 of 1024 MiB held, 0 pages swapped out since the last check" in (
        ping.body
    )


def test_ram_past_half_pings_fail(check):
    # Processes hold 2.5 GiB of 4: MemAvailable counts the page cache as free.
    result = check.run(meminfo(available=1536 * 1024))
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING + "/fail"
    assert "more than half the RAM" in ping.body
    assert "move up a server size" in ping.body


def test_exactly_half_is_not_past_half(check):
    result = check.run(meminfo(available=TOTAL // 2))
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING


def test_one_kilobyte_past_half_is(check):
    result = check.run(meminfo(available=TOTAL // 2 - 1))
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING + "/fail"


def test_swap_held_with_no_pages_swapped_out_since_is_fine(check):
    """The review's case: at swappiness 60 a healthy server holds idle pages in swap
    for good, and a check that failed on them would hide a real RAM alert."""
    assert check.run(meminfo(available=LOW), pswpout=500).returncode == 0
    result = check.run(meminfo(available=LOW, swap_free=SWAP - 2000), pswpout=500)
    assert result.returncode == 0, result.stderr
    assert [p.url for p in check.sent()] == [PING, PING]
    assert "swap: 1 of 1024 MiB held, 0 pages swapped out" in check.sent()[1].body


def test_the_first_run_only_records_the_counter(check):
    result = check.run(meminfo(available=LOW), pswpout=123456)
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING
    assert check.state.read_text() == "123456\n"


def test_pages_swapped_out_since_the_last_run_ping_fail_then_recover(check):
    assert check.run(meminfo(available=LOW), pswpout=100).returncode == 0
    result = check.run(meminfo(available=LOW), pswpout=164)
    assert result.returncode == 0, result.stderr
    assert check.run(meminfo(available=LOW), pswpout=164).returncode == 0
    first, swapped, quiet = check.sent()
    assert first.url == PING
    assert swapped.url == PING + "/fail"
    assert "the kernel swapped pages out" in swapped.body
    assert "64 pages swapped out since the last check" in swapped.body
    assert "more than half the RAM" not in swapped.body
    assert quiet.url == PING
    assert check.state.read_text() == "164\n"


def test_a_counter_reset_by_a_reboot_counts_as_nothing_swapped(check):
    assert check.run(meminfo(available=LOW), pswpout=90000).returncode == 0
    result = check.run(meminfo(available=LOW), pswpout=3)
    assert result.returncode == 0, result.stderr
    assert [p.url for p in check.sent()] == [PING, PING]
    assert check.state.read_text() == "3\n"


@pytest.mark.parametrize("kept", ["", "junk\n"])
def test_a_state_file_it_cant_read_counts_as_a_first_run(check, kept):
    check.state.write_text(kept)
    result = check.run(meminfo(available=LOW), pswpout=77)
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING
    assert check.state.read_text() == "77\n"


def test_both_at_once_names_both(check):
    assert check.run(meminfo(available=LOW), pswpout=1).returncode == 0
    result = check.run(meminfo(available=1024 * 1024, swap_free=0), pswpout=2)
    assert result.returncode == 0, result.stderr
    ping = check.sent()[-1]
    assert ping.url == PING + "/fail"
    assert "more than half the RAM" in ping.body
    assert "the kernel swapped pages out" in ping.body


def test_a_server_with_no_swap_is_fine(check):
    result = check.run(meminfo(available=LOW, swap_free=0, swap_total=0))
    assert result.returncode == 0, result.stderr
    [ping] = check.sent()
    assert ping.url == PING
    assert "swap: 0 of 0 MiB held" in ping.body


@pytest.mark.parametrize(
    "info",
    [
        None,
        "MemTotal: 4194304 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        "MemTotal: 4194304 kB\nMemAvailable: 1048576 kB\n",
        "MemTotal: 0 kB\nMemAvailable: 0 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
    ],
    ids=["no meminfo", "no MemAvailable", "no swap fields", "no RAM"],
)
def test_a_meminfo_it_cant_read_pings_fail_and_exits_1(check, info):
    result = check.run(info)
    assert result.returncode == 1
    [ping] = check.sent()
    assert ping.url == PING + "/fail"
    assert "couldn't read" in ping.body
    assert "couldn't read" in result.stderr


def test_a_vmstat_it_cant_read_pings_fail_and_exits_1(check):
    result = check.run(meminfo(available=LOW), pswpout=None)
    assert result.returncode == 1
    [ping] = check.sent()
    assert ping.url == PING + "/fail"
    assert "couldn't read" in ping.body
    assert "couldn't read" in result.stderr


def test_the_ping_url_never_reaches_a_command_line_or_the_output(check):
    result = check.run(meminfo(available=1024 * 1024))
    [ping] = check.sent()
    assert not any("hc-ping.com" in arg for arg in ping.args)
    assert PING not in result.stdout + result.stderr


def test_a_failed_ping_exits_1(check):
    check.status.write_text("7")
    result = check.run(meminfo(available=3 * 1024 * 1024))
    assert result.returncode == 1
    assert "ping" in result.stderr
    assert PING not in result.stderr


@pytest.mark.parametrize("url", ["", "https://example.com/x", PING + " x"])
def test_a_ping_url_that_isnt_one_is_refused_without_a_call(check, url):
    result = check.run(meminfo(available=3 * 1024 * 1024), MEMORY_PING=url)
    assert result.returncode == 1
    assert "MEMORY_PING" in result.stderr
    assert check.sent() == []


# memory.sh


@pytest.fixture
def box(tmp_path):
    root = tmp_path / "root"
    root.mkdir()
    fakes = tmp_path / "fakes"
    fakes.mkdir()
    log = tmp_path / "log"
    log.touch()
    for tool in ("systemctl", "chown"):
        executable(fakes / tool, f'#!/bin/sh\necho "{tool} $*" >> {log}\n')
    env = {"PATH": f"{fakes}:{os.environ['PATH']}", "FILEDRAWER_ROOT": str(root)}

    def install(ping: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["bash", str(INSTALL)],
            env={**env, "MEMORY_PING": ping},
            capture_output=True,
            text=True,
            check=False,
        )

    return SimpleNamespace(
        root=root,
        log=log,
        install=install,
        settings=root / "etc/filedrawer/memory.env",
        units=root / "etc/systemd/system",
    )


def unit_lines(path: Path) -> list[str]:
    return [line.strip() for line in path.read_text().splitlines()]


def test_the_ping_url_is_stored_for_roots_eyes_only(box):
    result = box.install(PING)
    assert result.returncode == 0, result.stderr
    assert box.settings.read_text() == f"MEMORY_PING={PING}\n"
    assert stat.S_IMODE(box.settings.stat().st_mode) == 0o600
    assert f"chown root:root {box.settings}" in box.log.read_text()
    assert PING not in result.stdout + result.stderr


def test_an_empty_url_keeps_the_one_stored(box):
    assert box.install(PING).returncode == 0
    result = box.install("")
    assert result.returncode == 0, result.stderr
    assert box.settings.read_text() == f"MEMORY_PING={PING}\n"


def test_a_new_url_replaces_the_one_stored(box):
    assert box.install(PING).returncode == 0
    result = box.install(OTHER_PING)
    assert result.returncode == 0, result.stderr
    assert box.settings.read_text() == f"MEMORY_PING={OTHER_PING}\n"


@pytest.mark.parametrize("url", ["", "https://example.com/x", "hc-ping.com/x"])
def test_a_url_it_cant_use_is_refused_naming_the_stage_not_the_url(box, url):
    result = box.install(url)
    assert result.returncode == 1
    assert "ops/provision-vps --ask-all memory" in result.stderr
    if url:
        assert url not in result.stderr
    assert not box.settings.exists()
    assert box.log.read_text() == ""


def test_the_timer_runs_the_check_every_5_minutes_and_runs_it_once_now(box):
    result = box.install(PING)
    assert result.returncode == 0, result.stderr
    timer = unit_lines(box.units / "filedrawer-memory-check.timer")
    assert "OnCalendar=*:0/5" in timer
    assert "WantedBy=timers.target" in timer
    log = box.log.read_text().splitlines()
    assert log.index("systemctl daemon-reload") < log.index(
        "systemctl enable --now filedrawer-memory-check.timer"
    )
    # The check reports at once, so the owner sees healthchecks.io's check go up.
    assert log[-1] == "systemctl start filedrawer-memory-check.service"


def test_the_check_runs_as_no_one_with_its_ping_url(box):
    result = box.install(PING)
    assert result.returncode == 0, result.stderr
    unit = unit_lines(box.units / "filedrawer-memory-check.service")
    for line in (
        "Type=oneshot",
        "DynamicUser=yes",
        # Required: a check that can't report would fail unseen.
        "EnvironmentFile=/etc/filedrawer/memory.env",
        "ExecStart=/usr/local/sbin/filedrawer-memory-check",
        "ProtectSystem=strict",
        # Where the check keeps the last run's pswpout.
        "StateDirectory=filedrawer-memory-check",
    ):
        assert line in unit, line
    # ProcSubset=pid would hide /proc/meminfo, the one file it reads.
    assert not any(line.startswith("ProcSubset=") for line in unit)


def test_a_rerun_writes_the_same_units(box):
    assert box.install(PING).returncode == 0
    first = {p.name: p.read_text() for p in box.units.iterdir()}
    result = box.install("")
    assert result.returncode == 0, result.stderr
    assert {p.name: p.read_text() for p in box.units.iterdir()} == first
