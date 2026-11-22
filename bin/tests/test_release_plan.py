"""Tests for bin/release-plan, which release-cli.yml runs before it publishes (#84):
`gate` says whether a run started by hand may release, and `plan` says
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


def pack(folder, name, version, normalized):
    """A .crate as cargo packs one: the files under NAME-VERSION/, Cargo.toml
    normalized and the original kept as Cargo.toml.orig."""
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
        add("Cargo.toml", normalized.encode())
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
    elif m := re.fullmatch(r"https://pypi\.org/pypi/filedrawer/([^/]+)/json", url):
        if m.group(1) in registries["pypi"]:
            files = registries.get("pypi_files", {}).get(m.group(1), 6)
            body = os.environ.get("FAKE_PYPI_VERSION") or json.dumps(
                {"urls": [{"filename": f"file-{n}"} for n in range(files)]}
            )
    elif url == "https://registry.npmjs.org/@filedrawer/cli":
        if registries["npm"]:
            optional = registries.get("npm_optional", {})
            body = json.dumps({"versions": {
                v: {"optionalDependencies": {f"@filedrawer/{p}": v for p in optional.get(v, [])}}
                for v in registries["npm"]
            }})
    elif m := re.fullmatch(r"https://registry\.npmjs\.org/@filedrawer/(cli-[^/]+)", url):
        versions = registries.get("npm_platforms", {}).get(m.group(1))
        if versions is not None:
            body = json.dumps({"versions": {v: {} for v in versions}})
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
    normalized = os.environ.get("FAKE_NORMALIZED", "# normalized by this cargo\n")
    (target / f"filedrawer-analyzer-{version}.crate").write_bytes(
        pack(folder, "filedrawer-analyzer", version, normalized)
    )
    sys.exit(0)

if tool == "gh":
    path = args[1]
    if "FAKE_GH_BODY" in os.environ and os.environ.get("FAKE_GH_PATH", "") in path:
        print(os.environ["FAKE_GH_BODY"])
        sys.exit(0)
    github = json.loads((state / "github.json").read_text())
    if path.endswith("/git/ref/heads/main"):
        print(json.dumps({"ref": "refs/heads/main", "object": {"sha": github["tip"]}}))
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
        older cargo would have normalized it, its manifest at VERSION."""
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
            env={**self.env, "FAKE_NORMALIZED": "# normalized by an older cargo\n"},
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

    def run(self, *args: str, **env: str):
        return subprocess.run(
            [str(SCRIPT), *args],
            cwd=self.repo,
            env={**self.env, **env},
            capture_output=True,
            text=True,
            check=False,
        )

    def gate(self, *args: str, **env: str):
        return subprocess.run(
            [str(SCRIPT), "gate", "--repo", SLUG, *args],
            cwd=self.repo,
            env={**self.env, **env},
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
        "pypi_files": "6",
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


def test_cargos_own_normalized_manifest_is_no_change(tmp_path):
    """A newer cargo writes Cargo.toml differently; Cargo.toml.orig is what's compared."""
    box = Box(tmp_path)
    box.released("0.1.0")
    assert (
        outputs(box.plan(FAKE_NORMALIZED="# normalized by a newer cargo\n"))[
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


def test_a_version_pypi_has_only_some_files_of_is_released_there_again(tmp_path):
    """A run that uploaded some wheels and then failed (#87): bin/pypi-release
    uploads the rest."""
    box = Box(tmp_path, cli="0.1.0")
    box.released("0.1.0")
    box.registries["pypi_files"] = {"0.1.0": 5}
    box.save()
    plan = outputs(box.plan())
    assert (plan["crates_cli"], plan["pypi"], plan["npm"], plan["build"]) == (
        "false",
        "true",
        "false",
        "true",
    )


@pytest.mark.parametrize("body", ["<html>", "{}", "[1]", '{"urls": 6}'])
def test_a_list_of_pypis_files_in_another_shape_stops_the_plan(tmp_path, body):
    box = Box(tmp_path, cli="0.1.0")
    box.released("0.1.0")
    done = box.plan(FAKE_PYPI_VERSION=body)
    assert done.returncode == 1, done.stdout + done.stderr
    assert "::error::" in done.stdout and "Traceback" not in done.stderr


def test_the_plans_count_of_pypi_files_is_the_workflows_wheels_and_the_sdist(tmp_path):
    workflow = (REPO / ".github/workflows/release-cli.yml").read_text()
    wheel_job = workflow.split("\n  wheel:\n")[1].split("\n  sdist:\n")[0]
    targets = [
        line for line in wheel_job.splitlines() if line.strip().startswith("target: ")
    ]
    box = Box(tmp_path)
    box.released("0.1.0")
    assert outputs(box.plan())["pypi_files"] == str(len(targets) + 1)


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


MAIN = ["--ref", "refs/heads/main", "--sha", TIP]


def test_a_run_from_mains_tip_releases_it_once_its_check_passed(tmp_path):
    """The rest of CI, such as a flaky smoke run, doesn't hold a release back."""
    box = Box(tmp_path)
    ci_run(box, 7, TIP)
    assert outputs(box.gate(*MAIN)) == {"sha": TIP}


def test_a_run_before_mains_check_passed_fails(tmp_path):
    box = Box(tmp_path)
    ci_run(box, 7, TIP, check="failure")
    done = box.gate(*MAIN)
    assert done.returncode == 1
    assert "::error::" in done.stdout and "wait for it" in done.stdout


@pytest.mark.parametrize("ref", ["refs/heads/fix", "refs/tags/main", "main"])
def test_a_run_started_from_another_ref_fails(tmp_path, ref):
    """The environments refuse it later anyway; this says why in seconds."""
    box = Box(tmp_path)
    ci_run(box, 7, TIP)
    done = box.gate("--ref", ref, "--sha", TIP)
    assert done.returncode == 1
    assert "::error::" in done.stdout and "ops/release-cli" in done.stdout


def test_a_run_of_a_commit_main_has_moved_past_fails(tmp_path):
    """Main moved between the dispatch and the gate: the registries' provenance
    would name the commit dispatched, not the one published."""
    box = Box(tmp_path)
    ci_run(box, 7, TIP)
    done = box.gate("--ref", "refs/heads/main", "--sha", "b" * 40)
    assert done.returncode == 1
    assert "::error::" in done.stdout and TIP in done.stdout


@pytest.mark.parametrize(
    ("path", "body"),
    [
        ("/git/ref/heads/main", "<html>"),
        ("/git/ref/heads/main", "{}"),
        ("/git/ref/heads/main", '{"object": 1}'),
        ("/actions/runs?", '{"runs": []}'),
        ("/actions/runs?", '{"workflow_runs": [{"id": 7}]}'),
        ("/jobs", '{"jobs": [{"name": "check"}]}'),
    ],
)
def test_a_github_answer_in_another_shape_fails_in_a_line(tmp_path, path, body):
    box = Box(tmp_path)
    ci_run(box, 7, TIP)
    done = box.gate(*MAIN, FAKE_GH_PATH=path, FAKE_GH_BODY=body)
    assert done.returncode == 1, done.stdout + done.stderr
    assert "::error::" in done.stdout and "Traceback" not in done.stderr


@pytest.mark.parametrize("args", [["--ref", "refs/heads/main"], ["--sha", TIP], []])
def test_the_gate_takes_a_ref_and_a_sha(tmp_path, args):
    done = Box(tmp_path).gate(*args)
    assert done.returncode == 2
    assert "required" in done.stderr


# analyzer


def test_an_analyzer_as_crates_io_has_it_is_unchanged(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    assert outputs(box.run("analyzer")) == {"changed": "false"}


def test_an_analyzer_that_differs_from_crates_ios_is_changed(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0")
    (box.repo / "crates/analyzer/src/lib.rs").write_text("pub fn changed() {}\n")
    assert outputs(box.run("analyzer")) == {"changed": "true"}


def test_an_analyzer_version_crates_io_lacks_is_unchanged_since_a_release_publishes_it(
    tmp_path,
):
    box = Box(tmp_path, analyzer="0.1.1", requirement="=0.1.1")
    box.released("0.1.0")
    (box.repo / "crates/analyzer/src/lib.rs").write_text("pub fn changed() {}\n")
    assert outputs(box.run("analyzer")) == {"changed": "false"}


def test_a_yanked_analyzer_cant_be_told(tmp_path):
    box = Box(tmp_path)
    box.released("0.1.0", yanked=True)
    done = box.run("analyzer")
    assert done.returncode == 1 and "yanked" in done.stdout


# published

PLATFORMS = ["cli-linux-x64", "cli-darwin-arm64"]


def everywhere(box: Box, cli: str) -> None:
    box.released(cli)
    box.registries["npm_optional"] = {cli: PLATFORMS}
    box.registries["npm_platforms"] = {p: [cli] for p in PLATFORMS}
    box.save()


def test_a_version_every_registry_has_is_published(tmp_path):
    box = Box(tmp_path, cli="0.1.0")
    everywhere(box, "0.1.0")
    done = box.run("published")
    assert done.returncode == 0, done.stdout + done.stderr


@pytest.mark.parametrize(
    ("missing", "named"),
    [
        (
            lambda r: r["crates"]["filedrawer"].pop("0.1.0"),
            "crates.io filedrawer 0.1.0",
        ),
        (
            lambda r: r["crates"]["filedrawer-analyzer"].pop("0.1.0"),
            "crates.io filedrawer-analyzer 0.1.0",
        ),
        (lambda r: r.update(pypi_files={"0.1.0": 5}), "PyPI filedrawer 0.1.0"),
        (lambda r: r.update(npm=["0.0.0"]), "npm @filedrawer/cli 0.1.0"),
        (
            lambda r: r["npm_platforms"].update({"cli-darwin-arm64": ["0.0.0"]}),
            "npm @filedrawer/cli-darwin-arm64 0.1.0",
        ),
    ],
)
def test_each_registry_that_lacks_the_version_is_named(tmp_path, missing, named):
    """npm answered 404 for some minutes after its job passed, on 2026-11-21."""
    box = Box(tmp_path, cli="0.1.0")
    everywhere(box, "0.1.0")
    missing(box.registries)
    box.save()
    done = box.run("published")
    assert done.returncode == 1, done.stdout + done.stderr
    assert done.stdout.splitlines() == [named]


def test_a_registry_that_doesnt_answer_is_couldnt_tell_not_missing(tmp_path):
    box = Box(tmp_path, cli="0.1.0")
    everywhere(box, "0.1.0")
    done = box.run("published", FAKE_DOWN="pypi.org")
    assert done.returncode == 3
    assert "::error::" in done.stdout


# ci


def test_ci_says_whether_check_passed_failed_or_is_pending(tmp_path):
    box = Box(tmp_path)
    assert outputs(box.run("ci", "--repo", SLUG, TIP))["state"] == "absent"
    box.github["runs"].append(
        {"id": 8, "name": "CI", "head_sha": TIP, "event": "push", "html_url": "u8"}
    )
    box.github["jobs"]["8"] = [{"name": "check", "conclusion": None}]
    box.save_github()
    assert outputs(box.run("ci", "--repo", SLUG, TIP)) == {
        "state": "pending",
        "url": "u8",
    }
    box.github["jobs"]["8"] = [{"name": "check", "conclusion": "failure"}]
    box.save_github()
    assert outputs(box.run("ci", "--repo", SLUG, TIP)) == {
        "state": "failed",
        "url": "u8",
    }
    ci_run(box, 9, TIP)
    assert outputs(box.run("ci", "--repo", SLUG, TIP))["state"] == "passed"


def test_help_says_what_it_does():
    done = subprocess.run(
        [str(SCRIPT), "-h"], capture_output=True, text=True, check=False
    )
    assert done.returncode == 0
    for command in ("plan", "gate", "analyzer", "published", "ci"):
        assert command in done.stdout
