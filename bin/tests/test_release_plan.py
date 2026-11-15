"""Tests for bin/release-plan, which release-cli.yml runs before it publishes (#84):
`gate` says whether a CI run may release and from which commit, and `plan` says
what each registry lacks, after the analyzer rule (design.md, Distribution).

They run it in a made-up repo with the two crates' Cargo.tomls, against fakes of
curl, answering crates.io, static.crates.io, PyPI and npm from a made-up state, of
cargo, whose `package` packs the analyzer's folder as cargo would, and of gh,
answering GitHub's API.
"""

import json
import os
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "bin/release-plan"
SLUG = "filedrawer-dev/filedrawer"
TIP = "a" * 40

FAKE = r'''#!/usr/bin/env python3
"""curl, cargo or gh, by the name it was called with, against $FAKE_STATE."""
import io, json, os, re, sys, tarfile
from pathlib import Path

state = Path(os.environ["FAKE_STATE"])
tool, args = Path(sys.argv[0]).name, sys.argv[1:]
with (state / "log").open("a") as log:
    log.write(" ".join([tool, *args]) + "\n")
registries = json.loads((state / "registries.json").read_text())


def pack(folder, name, version, normalised):
    """A .crate as cargo packs one: the files under NAME-VERSION/, Cargo.toml
    normalised and the original kept as Cargo.toml.orig."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as tar:
        def add(path, data):
            info = tarfile.TarInfo(f"{name}-{version}/{path}")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for file in sorted(folder.rglob("*")):
            if file.is_file():
                rel = file.relative_to(folder).as_posix()
                add("Cargo.toml.orig" if rel == "Cargo.toml" else rel, file.read_bytes())
        add("Cargo.toml", normalised.encode())
        add(".cargo_vcs_info.json", b'{"git": {"sha1": "x"}}')
    return buffer.getvalue()


if tool == "curl":
    url = args[-1]
    out = Path(args[args.index("-o") + 1])
    if os.environ.get("FAKE_DOWN") and os.environ["FAKE_DOWN"] in url:
        sys.stdout.write("503")
        sys.exit(0)
    body, status = None, 404
    if m := re.fullmatch(r"https://crates\.io/api/v1/crates/([^/]+)", url):
        versions = registries["crates"].get(m.group(1))
        if versions is not None:
            body = json.dumps({"versions": [{"num": v, "yanked": y} for v, y in versions.items()]})
    elif m := re.fullmatch(r"https://static\.crates\.io/crates/([^/]+)/([^/]+)-([^/]+)\.crate", url):
        crate = state / "static" / f"{m.group(1)}-{m.group(3)}.crate"
        if crate.exists():
            out.write_bytes(crate.read_bytes())
            status = 200
    elif url == "https://pypi.org/pypi/filedrawer/json":
        if registries["pypi"]:
            body = json.dumps({"releases": {v: [] for v in registries["pypi"]}})
    elif url == "https://registry.npmjs.org/@filedrawer/cli":
        if registries["npm"]:
            body = json.dumps({"versions": {v: {} for v in registries["npm"]}})
    else:
        sys.exit(6)
    if body is not None:
        out.write_text(body)
        status = 200
    sys.stdout.write(str(status))
    sys.exit(0)

if tool == "cargo":
    assert args[:3] == ["package", "-p", "filedrawer-analyzer"], args
    if os.environ.get("FAKE_PACKAGE_FAILS"):
        sys.exit(101)
    folder = Path("crates/analyzer")
    version = re.search(r'^version = "(.*)"$', (folder / "Cargo.toml").read_text(), re.M).group(1)
    target = Path("target/package")
    target.mkdir(parents=True, exist_ok=True)
    normalised = os.environ.get("FAKE_NORMALISED", "# normalised by this cargo\n")
    (target / f"filedrawer-analyzer-{version}.crate").write_bytes(
        pack(folder, "filedrawer-analyzer", version, normalised)
    )
    sys.exit(0)

if tool == "gh":
    path = args[1]
    github = json.loads((state / "github.json").read_text())
    if path.endswith("/commits/main"):
        print(json.dumps({"sha": github["tip"]}))
    elif m := re.search(r"/actions/runs/(\d+)/jobs$", path):
        print(json.dumps({"jobs": github["jobs"].get(m.group(1), [])}))
    elif "/actions/runs?" in path:
        sha = re.search(r"head_sha=(\w+)", path).group(1)
        runs = [r for r in github["runs"] if r["head_sha"] == sha]
        print(json.dumps({"workflow_runs": runs}))
    else:
        sys.exit(1)
    sys.exit(0)
'''


class Box:
    def __init__(
        self,
        root: Path,
        *,
        cli: str = "0.2.0",
        analyzer: str = "0.1.0",
        requirement: str = "=0.1.0",
    ):
        self.root = root
        self.repo = root / "repo"
        self.state = root / "state"
        (self.state / "static").mkdir(parents=True)
        (self.state / "log").touch()
        fakes = root / "fakes"
        fakes.mkdir()
        for tool in ("curl", "cargo", "gh"):
            (fakes / tool).write_text(FAKE)
            (fakes / tool).chmod(0o755)
        self.env = {
            **os.environ,
            "PATH": f"{fakes}{os.pathsep}{os.environ['PATH']}",
            "FAKE_STATE": str(self.state),
        }
        analyzer_dir = self.repo / "crates/analyzer"
        (analyzer_dir / "src").mkdir(parents=True)
        (analyzer_dir / "src/lib.rs").write_text("pub fn read() {}\n")
        (analyzer_dir / "README.md").write_text("# filedrawer-analyzer\n")
        self.set_analyzer(analyzer)
        (self.repo / "crates/cli").mkdir(parents=True)
        self.set_cli(cli, requirement)
        self.registries = {"crates": {}, "pypi": [], "npm": []}
        self.save()
        self.github = {"tip": TIP, "jobs": {}, "runs": []}
        self.save_github()

    def set_analyzer(self, version: str) -> None:
        (self.repo / "crates/analyzer/Cargo.toml").write_text(
            f'[package]\nname = "filedrawer-analyzer"\nversion = "{version}"\n'
        )

    def set_cli(self, version: str, requirement: str) -> None:
        (self.repo / "crates/cli/Cargo.toml").write_text(
            f'[package]\nname = "filedrawer"\nversion = "{version}"\n\n'
            f'[dependencies]\nfiledrawer-analyzer = {{ version = "{requirement}", path = "../analyzer" }}\n'
        )

    def save(self) -> None:
        (self.state / "registries.json").write_text(json.dumps(self.registries))

    def save_github(self) -> None:
        (self.state / "github.json").write_text(json.dumps(self.github))

    def released(self, cli: str, analyzer: str = "0.1.0", yanked: bool = False) -> None:
        """Every registry has CLI, and crates.io has ANALYZER as this repo packs it now."""
        self.registries["crates"].setdefault("filedrawer", {})[cli] = False
        self.registries["crates"].setdefault("filedrawer-analyzer", {})[analyzer] = (
            yanked
        )
        self.registries["pypi"].append(cli)
        self.registries["npm"] = [*self.registries["npm"], "0.0.0", cli]
        self.save()
        self.publish_analyzer_as_it_is(analyzer)

    def publish_analyzer_as_it_is(self, version: str) -> None:
        """Puts the repo's analyzer on the fake static.crates.io at VERSION, as an
        older cargo would have normalised it, its manifest at VERSION."""
        manifest = self.repo / "crates/analyzer/Cargo.toml"
        current = manifest.read_text()
        self.set_analyzer(version)
        done = subprocess.run(
            [
                "cargo",
                "package",
                "-p",
                "filedrawer-analyzer",
                "--no-verify",
                "--allow-dirty",
            ],
            cwd=self.repo,
            env={**self.env, "FAKE_NORMALISED": "# normalised by an older cargo\n"},
            check=True,
        )
        assert done.returncode == 0
        manifest.write_text(current)
        built = self.repo / f"target/package/filedrawer-analyzer-{version}.crate"
        (self.state / "static" / built.name).write_bytes(built.read_bytes())
        built.unlink()
        (self.state / "log").write_text("")

    def plan(self, *args: str, **env: str):
        return subprocess.run(
            [str(SCRIPT), "plan", *args],
            cwd=self.repo,
            env={**self.env, **env},
            capture_output=True,
            text=True,
            check=False,
        )

    def gate(self, *args: str):
        return subprocess.run(
            [str(SCRIPT), "gate", "--repo", SLUG, *args],
            cwd=self.repo,
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )


def outputs(done: subprocess.CompletedProcess) -> dict[str, str]:
    assert done.returncode == 0, done.stdout + done.stderr
    return dict(
        line.split("=", 1)
        for line in done.stdout.splitlines()
        if "=" in line and not line.startswith("::")
    )


# plan


def test_a_new_cli_version_goes_to_every_registry_and_an_unchanged_analyzer_to_none(
    tmp_path,
):
    box = Box(tmp_path)
    box.released("0.1.0")
    plan = outputs(box.plan())
    assert plan == {
        "cli_version": "0.2.0",
        "analyzer_version": "0.1.0",
        "crates_analyzer": "false",
        "crates_cli": "true",
        "pypi": "true",
        "npm": "true",
        "build": "true",
    }


def test_with_every_registry_at_the_cli_version_nothing_is_released_or_checked(
    tmp_path,
):
    """Between releases the analyzer may change freely (the owner, 2026-11-14)."""
    box = Box(tmp_path, cli="0.1.0")
    box.released("0.1.0")
    (box.repo / "crates/analyzer/src/lib.rs").write_text("pub fn changed() {}\n")
    plan = outputs(box.plan())
    assert {
        plan[k] for k in ("crates_analyzer", "crates_cli", "pypi", "npm", "build")
    } == {"false"}
    assert not [
        c for c in (box.state / "log").read_text().splitlines() if c.startswith("cargo")
    ]


def test_a_rerun_releases_only_to_the_registries_that_lack_the_version(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    box.registries["crates"]["filedrawer"]["0.2.0"] = False
    box.registries["pypi"].append("0.2.0")
    box.save()
    plan = outputs(box.plan())
    assert (plan["crates_cli"], plan["pypi"], plan["npm"], plan["build"]) == (
        "false",
        "false",
        "true",
        "true",
    )


def test_a_bumped_analyzer_is_published_first(tmp_path):
    box = Box(tmp_path, analyzer="0.1.1", requirement="=0.1.1")
    box.released("0.1.0")
    (box.repo / "crates/analyzer/src/lib.rs").write_text("pub fn changed() {}\n")
    plan = outputs(box.plan())
    assert (plan["analyzer_version"], plan["crates_analyzer"], plan["crates_cli"]) == (
        "0.1.1",
        "true",
        "true",
    )


def test_an_analyzer_changed_at_a_published_version_stops_the_release(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    (box.repo / "crates/analyzer/src/lib.rs").write_text("pub fn changed() {}\n")
    done = box.plan()
    assert done.returncode == 1
    assert "src/lib.rs" in done.stdout
    assert "bump" in done.stdout and "crates/analyzer" in done.stdout


def test_a_readme_counts_as_a_change_since_crates_io_shows_it(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    (box.repo / "crates/analyzer/README.md").write_text(
        "# filedrawer-analyzer, reworded\n"
    )
    done = box.plan()
    assert done.returncode == 1
    assert "README.md" in done.stdout


def test_a_new_file_counts_as_a_change(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    (box.repo / "crates/analyzer/src/extra.rs").write_text("\n")
    done = box.plan()
    assert done.returncode == 1
    assert "src/extra.rs" in done.stdout


def test_cargos_own_normalised_manifest_is_no_change(tmp_path):
    """A newer cargo writes Cargo.toml differently; Cargo.toml.orig is what's compared."""
    box = Box(tmp_path)
    box.released("0.1.0")
    assert (
        outputs(box.plan(FAKE_NORMALISED="# normalised by a newer cargo\n"))[
            "crates_cli"
        ]
        == "true"
    )


def test_a_requirement_that_isnt_the_analyzers_exact_version_stops_the_release(
    tmp_path,
):
    box = Box(tmp_path, requirement="0.1.0")
    box.released("0.1.0")
    done = box.plan()
    assert done.returncode == 1
    assert '"=0.1.0"' in done.stdout


def test_a_yanked_analyzer_stops_the_release(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0", yanked=True)
    done = box.plan()
    assert done.returncode == 1
    assert "yanked" in done.stdout


def test_a_version_below_one_a_registry_has_stops_the_release(tmp_path):
    """A re-run of an old CI run would publish backwards and move npm's latest."""
    box = Box(tmp_path, cli="0.1.5")
    box.released("0.2.0")
    done = box.plan()
    assert done.returncode == 1
    assert "0.2.0" in done.stdout


def test_npms_placeholders_and_staged_versions_arent_a_higher_version(tmp_path):
    box = Box(tmp_path, cli="0.1.0")
    box.released("0.0.9")
    box.registries["npm"] = ["0.0.0", "0.0.0-stage"]
    box.save()
    assert outputs(box.plan())["npm"] == "true"


@pytest.mark.parametrize(
    "host", ["crates.io/api", "static.crates.io", "pypi.org", "registry.npmjs.org"]
)
def test_a_registry_that_doesnt_answer_stops_the_plan(tmp_path, host):
    box = Box(tmp_path)
    box.released("0.1.0")
    done = box.plan(FAKE_DOWN=host)
    assert done.returncode == 1
    assert "::error::" in done.stdout


def test_a_crate_crates_io_lists_but_doesnt_serve_yet_is_an_error_not_a_change(
    tmp_path,
):
    box = Box(tmp_path)
    box.released("0.1.0")
    for crate in (box.state / "static").iterdir():
        crate.unlink()
    done = box.plan()
    assert done.returncode == 1
    assert "rerun" in done.stdout
    assert "bump" not in done.stdout


def test_a_dispatched_version_must_be_the_clis(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    done = box.plan("--version", "0.3.0")
    assert done.returncode == 1
    assert "0.2.0" in done.stdout
    assert outputs(box.plan("--version", "0.2.0"))["crates_cli"] == "true"


def test_a_pre_release_version_is_refused_in_a_line(tmp_path):
    box = Box(tmp_path, cli="0.2.0-rc.1")
    box.released("0.1.0")
    done = box.plan()
    assert done.returncode == 1
    assert "X.Y.Z" in done.stdout
    assert "Traceback" not in done.stderr


def test_a_cargo_package_that_fails_is_refused_in_a_line(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    done = box.plan(FAKE_PACKAGE_FAILS="1")
    assert done.returncode == 1
    assert "cargo package" in done.stdout
    assert "Traceback" not in done.stderr


def test_a_requirement_written_path_first_is_read(tmp_path):
    box = Box(tmp_path)
    (box.repo / "crates/cli/Cargo.toml").write_text(
        '[package]\nname = "filedrawer"\nversion = "0.2.0"\n\n'
        '[dependencies]\nfiledrawer-analyzer = { path = "../analyzer", version = "=0.1.0" }\n'
    )
    box.released("0.1.0")
    assert outputs(box.plan())["crates_cli"] == "true"


# gate


def ci_run(box: Box, run: int, sha: str, check: str = "success") -> None:
    box.github["runs"].append(
        {"id": run, "name": "CI", "head_sha": sha, "event": "push"}
    )
    box.github["jobs"][str(run)] = [
        {"name": "check", "conclusion": check},
        {"name": "smoke-staging", "conclusion": "failure"},
    ]
    box.save_github()


def test_a_ci_run_of_mains_tip_whose_check_passed_may_release(tmp_path):
    """The rest of CI, such as a flaky smoke run, doesn't hold a release back."""
    box = Box(tmp_path)
    ci_run(box, 7, TIP)
    assert outputs(box.gate("--run", "7", "--sha", TIP)) == {"go": "true", "sha": TIP}


def test_a_ci_run_whose_check_failed_releases_nothing(tmp_path):
    box = Box(tmp_path)
    ci_run(box, 7, TIP, check="failure")
    gate = outputs(box.gate("--run", "7", "--sha", TIP))
    assert gate["go"] == "false"


def test_a_ci_run_of_a_commit_main_has_moved_past_releases_nothing(tmp_path):
    """The later commit's run releases instead; an old run re-run can't."""
    box = Box(tmp_path)
    ci_run(box, 7, "b" * 40)
    gate = outputs(box.gate("--run", "7", "--sha", "b" * 40))
    assert gate["go"] == "false"


def test_a_dispatch_releases_mains_tip_once_its_check_passed(tmp_path):
    box = Box(tmp_path)
    ci_run(box, 7, TIP)
    assert outputs(box.gate("--dispatch")) == {"go": "true", "sha": TIP}


def test_a_dispatch_before_mains_check_passed_fails(tmp_path):
    box = Box(tmp_path)
    ci_run(box, 7, TIP, check="failure")
    done = box.gate("--dispatch")
    assert done.returncode == 1
    assert "::error::" in done.stdout


@pytest.mark.parametrize(
    "args",
    [["--run", "7"], ["--sha", TIP], [], ["--dispatch", "--run", "7", "--sha", TIP]],
)
def test_the_gate_takes_a_run_and_its_sha_or_a_dispatch(tmp_path, args):
    done = Box(tmp_path).gate(*args)
    assert done.returncode == 2
    assert "--dispatch" in done.stderr


def test_help_says_what_it_does():
    done = subprocess.run(
        [str(SCRIPT), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    assert "plan" in done.stdout and "gate" in done.stdout
