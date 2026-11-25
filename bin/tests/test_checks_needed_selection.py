"""bin/checks-needed --bin-tests on random reach graphs: which bin test files a change
to one path outside bin/ and ops/ selects, against a plain search over every chain of
references. The graphs skip the shell and Python parsers, whose kinds of reference
bin/tests/test_agent_pre_commit.py covers one by one: each file's references are given
straight to checks-needed's Tree."""

import importlib.machinery
import importlib.util
import random
from pathlib import Path, PurePosixPath

from hypothesis import HealthCheck, example, given, settings
from hypothesis import strategies as st

SCRIPT = Path(__file__).resolve().parents[1] / "checks-needed"
loader = importlib.machinery.SourceFileLoader("checks_needed", str(SCRIPT))
spec = importlib.util.spec_from_loader("checks_needed", loader)
cn = importlib.util.module_from_spec(spec)
loader.exec_module(cn)

EVERY = ["bin/tests"]


def within(path, ref):
    return path == ref or path.startswith(ref + "/")


def is_test(path):
    return within(path, "bin/tests") and PurePosixPath(path).name.startswith("test_")


@st.composite
def graphs(draw):
    """Files and the references each makes, and one changed path outside bin/ and
    ops/: test files and helpers in bin/tests, scripts in bin/, ops/ and lib/, data
    files in data/ and lib/. A reference names a script or data file, or a folder
    holding some."""
    tests = [f"bin/tests/test_{n}.py" for n in range(draw(st.integers(1, 8)))]
    helpers = [f"bin/tests/helper_{n}.py" for n in range(draw(st.integers(0, 2)))]
    scripts = [
        f"{draw(st.sampled_from(['bin', 'ops', 'lib']))}/s{n}"
        for n in range(draw(st.integers(0, 6)))
    ]
    data = [
        f"{draw(st.sampled_from(['data', 'lib', 'data/deep']))}/d{n}.toml"
        for n in range(draw(st.integers(1, 6)))
    ]
    targets = sorted(set(scripts + data))
    folders = sorted({str(PurePosixPath(t).parent) for t in targets} - {"bin", "ops"})
    choices = targets + folders
    refs = {}
    for path in tests + helpers + scripts:
        named = draw(st.lists(st.sampled_from(choices), max_size=3, unique=True))
        refs[path] = [ref for ref in named if ref != path]
    for path in data:
        refs[path] = []
    # A changed script is the rarer, harder case, so it's drawn as often as data.
    lib_scripts = [s for s in scripts if s.startswith("lib/")]
    pool = lib_scripts if lib_scripts and draw(st.booleans()) else data
    changed = draw(st.sampled_from(pool))
    return refs, changed


def selection(refs, changed, order):
    """What select_bin_tests answers, its Tree's paths and each file's references in
    the shuffled `order`."""
    tree = object.__new__(cn.Tree)
    tree.root = Path("/nonexistent")
    tree.paths = order.sample(sorted(refs), len(refs))
    tree.refs = {
        path: {ref: f"{path}:1: {ref}" for ref in order.sample(named, len(named))}
        for path, named in refs.items()
    }
    starts = {ref: cn.Step(f"{ref}: bin/tests covers it") for ref in cn.BIN_TESTS_PATHS}
    checks = {
        "bin-tests": cn.Inputs(tree, starts),
        "rust": cn.Inputs(tree, {}),
        "skills": cn.Inputs(tree, {}),
        "routes": cn.Inputs(tree, {}),
    }
    return cn.select_bin_tests(checks, [changed])


def read_files(refs):
    """The files whose references count: those under bin/ or ops/, and, in turn,
    those under a path a counted file names."""
    reached = {"bin", "ops"}
    read = set()
    while True:
        more = {f for f in refs if f not in read and any(within(f, r) for r in reached)}
        if not more:
            return read
        read |= more
        reached |= {ref for f in more for ref in refs[f]}


def expected(refs, changed):
    """The selection a search over every chain gives: the test files with a chain of
    references to `changed`, the first step a file or folder holding it and each step
    after naming a script itself; the whole suite when a helper beside the tests is on
    a chain, or a script on one is run by no test, through any walk of names; none
    when nothing reaches it."""
    read = read_files(refs)

    def holders(target, exact):
        return {
            f
            for f in read
            for ref in refs[f]
            if (ref == target if exact else within(target, ref))
        }

    # Every file with a chain to `changed`, a depth-first walk up the names.
    on_chain, todo = set(), list(holders(changed, exact=False))
    while todo:
        holder = todo.pop()
        # The changed file naming what reads it makes no chain to itself.
        if holder in on_chain or holder == changed:
            continue
        on_chain.add(holder)
        if not within(holder, "bin/tests"):
            todo.extend(holders(holder, exact=True))
    if not on_chain:
        return []
    if any(within(f, "bin/tests") and not is_test(f) for f in on_chain):
        return EVERY

    def run_by_a_test(script):
        """Whether some test names `script`, or a script some test runs does."""
        seen, todo = set(), [script]
        while todo:
            name = todo.pop()
            for holder in holders(name, exact=True) - seen:
                if is_test(holder):
                    return True
                seen.add(holder)
                todo.append(holder)
        return False

    scripts = {f for f in on_chain if not within(f, "bin/tests")}
    if not all(run_by_a_test(script) for script in scripts):
        return EVERY
    return sorted(f for f in on_chain if is_test(f))


# Each run reads the references in a shuffled order and is held to a search that
# doesn't depend on any, so the answer can't depend on the order either.
@settings(max_examples=100, deadline=None, suppress_health_check=[HealthCheck.too_slow])
@given(graphs(), st.randoms(use_true_random=False))
# A test runs ops/s2, which runs the changed lib/s1, which runs ops/s3, named by
# nothing else: s3 is tested through the changed script, so the selection is narrow.
@example(
    (
        {
            "bin/tests/test_0.py": ["bin/s0", "ops/s2"],
            "bin/s0": [],
            "lib/s1": ["ops/s3"],
            "ops/s2": ["bin/s0", "lib/s1"],
            "ops/s3": ["lib/s1"],
            "data/d0.toml": [],
        },
        "lib/s1",
    ),
    random.Random(0),
)
def test_the_selection_matches_a_search_of_every_chain(graph, order):
    refs, changed = graph
    assert selection(refs, changed, order) == expected(refs, changed)
