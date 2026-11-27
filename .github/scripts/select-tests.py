#!/usr/bin/env python3
"""Decide which tests the `Test Rust` job runs, from the files a pull request
changes.

    python3 .github/scripts/select-tests.py [--base REV] [--head REV]
                                            [--files-from FILE] [--out FILE]

A pull request's run tests the crates its diff can reach; a push to `main`
(and a tag, the nightly, a manual run) tests everything. So every commit on
`main` still gets the full suite, after the merge instead of before it.

Why: the full run holds a Docker host slot for 30-60 minutes, mostly in
`mongodb_tests`, and the host finishes about two of those an hour (see
docker-host-slot.sh). Most pull requests cannot affect most of it: a change
under `crates/hts` cannot break `helios-persistence`'s MongoDB suite.

What is selected, for a pull request:

  * A changed file belongs to the workspace package whose directory holds it.
    That package and everything that depends on it, directly or not, is
    "affected". Dependencies come from `cargo metadata` (normal, dev and build
    dependencies alike). One more case cargo cannot see: a crate whose
    sources name a path inside another crate (`"../hfs/..."`,
    `include_str!("../../ui/...")`) is affected when a file at or under
    that path changes.
  * The integration test targets (`tests/*.rs`) of affected packages run.
    Unit tests (`--lib --bins`) and doc-tests always run for every package:
    cargo can only narrow those with `-p`, which changes feature resolution
    and rebuilds the workspace (see test-rust.sh), and they take about five
    minutes.
  * If none of the selected targets starts a container, the job takes no
    Docker host slot, and the library modules that start containers in their
    unit tests are skipped (the same classification as docker-free-tests.py).
  * A changed file outside every package means the full run, unless it is on
    the short list below of paths no Rust test reads. So do: a diff that
    cannot be computed, a pull request labelled `full-ci`, and any error in
    this script. When in doubt it is the full run.

The build step is not narrowed: `cargo test --workspace --all-features
--no-run` still compiles every test target, so a change that breaks another
crate's tests at compile time fails the pull request as before.

Writes the plan as `key=value` lines to `--out` (read by test-rust.sh), the
same keys to `$GITHUB_OUTPUT`, and a summary to `$GITHUB_STEP_SUMMARY`.
"""

from __future__ import annotations

import argparse
import fnmatch
import importlib.util
import json
import os
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FULL_LABEL = "full-ci"
MONGO_LANE = "mongodb_tests"

# Paths outside every package that no Rust test reads. Everything else outside
# a package (Cargo.toml, Cargo.lock, data/, locales/, docker/, scripts/,
# .github/actions, this workflow and its scripts, ...) means the full run.
IGNORED = [
    "book/*",
    ".claude/*",
    ".agents/*",
    ".hfs-monitor/*",
    ".github/ISSUE_TEMPLATE/*",
    ".github/dependabot.yml",
    ".github/workflows/*",
    ".github/scripts/fhir-bench/*",
    ".github/scripts/obs-ab/*",
    "*.md",
    "*.png",
    "Dockerfile",
    ".dockerignore",
    "todo.txt",
]
# ...except this workflow, which is what runs the tests.
NOT_IGNORED = [".github/workflows/ci.yml"]

# A string literal that climbs out of its directory: "../x", "/../x".
ESCAPING = re.compile(r'"/?((?:\.\./)+[^"\s]*)"')


def load_classifier():
    """docker-free-tests.py, for which targets and modules start containers."""
    path = Path(__file__).with_name("docker-free-tests.py")
    spec = importlib.util.spec_from_file_location("docker_free_tests", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    ).stdout


def changed_files(base: str, head: str) -> list[str]:
    # --no-renames: a moved file counts at both its old and its new path.
    out = git("diff", "--name-only", "--no-renames", base, head)
    return [line for line in out.splitlines() if line]


def pr_labels() -> list[str] | None:
    """The pull request's labels, read now rather than from the event payload,
    so adding `full-ci` and re-running the job is enough. None if unknown."""
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    token = os.environ.get("GITHUB_TOKEN")
    repo = os.environ.get("GITHUB_REPOSITORY")
    if not event_path or not Path(event_path).is_file():
        return None
    event = json.loads(Path(event_path).read_text(encoding="utf-8"))
    pr = event.get("pull_request") or {}
    number = pr.get("number")
    payload = [label.get("name", "") for label in pr.get("labels", [])]
    if not (token and repo and number):
        return payload
    api = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    req = urllib.request.Request(
        f"{api}/repos/{repo}/issues/{number}/labels?per_page=100",
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=20) as resp:
            return [label.get("name", "") for label in json.load(resp)]
    except Exception as e:  # noqa: BLE001 - any failure falls back to the payload
        print(f"Could not read the pull request's labels ({e}); using the event payload.")
        return payload


class Workspace:
    def __init__(self) -> None:
        meta = json.loads(
            subprocess.run(
                ["cargo", "metadata", "--no-deps", "--format-version", "1"],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            ).stdout
        )
        members = set(meta["workspace_members"])
        self.packages = [p for p in meta["packages"] if p["id"] in members]
        self.names = {p["name"] for p in self.packages}
        # Package directory, relative to the repository root, with a trailing
        # slash: "crates/rest/".
        self.dirs = {
            p["name"]: Path(p["manifest_path"]).parent.resolve().relative_to(ROOT).as_posix() + "/"
            for p in self.packages
        }
        # deps[a] = the workspace packages a depends on.
        self.deps: dict[str, set[str]] = {
            p["name"]: {d["name"] for d in p["dependencies"] if d["name"] in self.names}
            for p in self.packages
        }
        # reads[a] = the paths inside other packages that a's sources name,
        # where a does not already depend on that package.
        self.reads: dict[str, set[str]] = {p["name"]: set() for p in self.packages}
        self.path_edges: list[tuple[str, str, str]] = []
        self._add_path_edges()

    def owner(self, path: str) -> str | None:
        """The package whose directory holds `path` (the deepest one)."""
        best = None
        for name, d in self.dirs.items():
            if path.startswith(d) and (best is None or len(d) > len(self.dirs[best])):
                best = name
        return best

    def _add_path_edges(self) -> None:
        for pkg in self.packages:
            name = pkg["name"]
            pkg_dir = ROOT / self.dirs[name]
            for f in pkg_dir.rglob("*.rs"):
                rel = f.relative_to(pkg_dir).parts
                if rel[0] == "target":
                    continue
                try:
                    text = f.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                if "../" not in text:
                    continue
                for m in ESCAPING.finditer(text):
                    # Relative to the file (include_str!) or to the package
                    # root (CARGO_MANIFEST_DIR); take whichever exists.
                    for origin in (f.parent, pkg_dir):
                        target = Path(os.path.normpath(origin / m.group(1)))
                        try:
                            rel_target = target.relative_to(ROOT).as_posix()
                        except ValueError:
                            continue
                        if not target.exists():
                            continue
                        other = self.owner(rel_target + ("/" if target.is_dir() else ""))
                        if other and other != name and other not in self.deps[name]:
                            self.reads[name].add(rel_target)
                            self.path_edges.append((name, other, f.relative_to(ROOT).as_posix()))

    def affected(self, changed: set[str], files: list[str]) -> set[str]:
        """`changed`, the packages that name one of the changed `files` (or a
        directory above it) by path, and every package that depends on one
        of those."""
        out = set(changed)
        for name, paths in self.reads.items():
            if any(f == p or f.startswith(p + "/") for p in paths for f in files):
                out.add(name)
        grew = True
        while grew:
            grew = False
            for name, deps in self.deps.items():
                if name not in out and deps & out:
                    out.add(name)
                    grew = True
        return out


def ignored(path: str) -> bool:
    if path in NOT_IGNORED:
        return False
    # fnmatch's `*` crosses `/`, so "*.md" would match crates/x/README.md;
    # those never get here (they belong to a package), and "book/*" is meant
    # to match the whole tree.
    return any(fnmatch.fnmatchcase(path, pat) for pat in IGNORED)


def plan(ws: Workspace, files: list[str] | None, reason_full: str | None) -> dict:
    cls = load_classifier()
    targets: dict[str, list[tuple[str, bool]]] = {}  # test name -> [(package, docker)]
    skips: list[str] = []
    for pkg in ws.packages:
        for t in pkg["targets"]:
            root = Path(t["src_path"])
            if "test" in t["kind"]:
                docker = cls.needs_docker(cls.module_files(root))
                targets.setdefault(t["name"], []).append((pkg["name"], docker))
            elif {"lib", "rlib", "cdylib"} & set(t["kind"]):
                skips += cls.lib_skips(root, cls.module_files(root))
    skips = sorted(set(skips))

    changed_pkgs: set[str] = set()
    untracked: list[str] = []
    if reason_full is None:
        for path in files or []:
            owner = ws.owner(path)
            if owner:
                changed_pkgs.add(owner)
            elif not ignored(path):
                untracked.append(path)
        if untracked:
            shown = ", ".join(untracked[:5]) + (" ..." if len(untracked) > 5 else "")
            reason_full = f"changed outside any crate: {shown}"

    if reason_full is not None:
        return {
            "mode": "full",
            "reason": reason_full,
            "docker": True,
            "mongo": True,
            "persistence": True,
            "tests": [],
            "skips": [],
            "affected": sorted(ws.names),
            "changed": sorted(changed_pkgs),
            "skipped_docker_targets": [],
        }

    affected = ws.affected(changed_pkgs, files or [])
    # `--test <name>` runs that name in every package that has it, so a name
    # is selected if any package with it is affected, and needs Docker if any
    # package's target of that name does.
    selected = sorted(n for n, owners in targets.items() if any(p in affected for p, _ in owners))
    docker = any(d for n in selected for _, d in targets[n])
    not_run = sorted(
        n for n, owners in targets.items() if n not in selected and any(d for _, d in owners)
    )
    return {
        "mode": "selected",
        "reason": "",
        "docker": docker,
        "mongo": MONGO_LANE in selected,
        "persistence": "helios-persistence" in affected,
        "tests": [n for n in selected if n != MONGO_LANE],
        # With containers in play the library tests run whole, as in the full
        # run; without, the modules that start containers are skipped.
        "skips": [] if docker else skips,
        "affected": sorted(affected),
        "changed": sorted(changed_pkgs),
        "skipped_docker_targets": not_run,
        "total_targets": len(targets),
    }


def write(p: dict, out: Path | None) -> None:
    b = lambda v: "true" if v else "false"  # noqa: E731
    lines = [
        f"mode={p['mode']}",
        f"docker={b(p['docker'])}",
        f"mongo={b(p['mongo'])}",
        f"persistence={b(p['persistence'])}",
        "tests=" + " ".join(p["tests"]),
        "skips=" + " ".join(p["skips"]),
    ]
    if out:
        out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as f:
            # Not the two lists: only the job's `if:` conditions read these.
            f.write("\n".join(lines[:4]) + "\n")

    if p["mode"] == "full":
        text = [f"### Test selection: full run", "", f"Reason: {p['reason']}."]
    else:
        n = len(p["tests"]) + (1 if p["mongo"] else 0)
        text = [
            "### Test selection: affected crates only",
            "",
            "| | |",
            "|---|---|",
            f"| Changed crates | {', '.join(p['changed']) or 'none'} |",
            f"| Affected (changed + dependents) | {', '.join(p['affected']) or 'none'} |",
            f"| Integration test targets run | {n} of {p['total_targets']} |",
            f"| `mongodb_tests` | {'runs' if p['mongo'] else 'not run'} |",
            f"| Docker host slot | {'taken' if p['docker'] else 'not needed'} |",
            "",
            "Unit tests and doc-tests run for every crate"
            + ("." if p["docker"] else ", minus the library modules that start containers."),
            f"The full suite runs on `main` after the merge. Label the pull request "
            f"`{FULL_LABEL}` and re-run this job to run it here.",
        ]
    print("\n".join(text))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("\n".join(text) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--base", default="HEAD^1", help="diff base (default: the merge commit's first parent)")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--files-from", type=Path, help="read the changed paths from a file instead of git")
    ap.add_argument("--out", type=Path, help="write the plan here, for test-rust.sh")
    ap.add_argument("--explain", action="store_true", help="also print the dependency edges found in sources")
    args = ap.parse_args()

    ws = Workspace()
    if args.explain:
        for a, b, f in sorted(set(ws.path_edges)):
            print(f"path edge: {a} -> {b} ({f})")

    reason = None
    files: list[str] | None = None
    event = os.environ.get("GITHUB_EVENT_NAME", "")
    if args.files_from:
        files = [l.strip() for l in args.files_from.read_text(encoding="utf-8").splitlines() if l.strip()]
    elif event and event != "pull_request":
        reason = f"a `{event}` run, not a pull request"
    else:
        labels = pr_labels() if event else []
        if labels and FULL_LABEL in labels:
            reason = f"the pull request is labelled `{FULL_LABEL}`"
        else:
            try:
                if event:
                    # refs/pull/<n>/merge: parent 1 is the base branch, parent
                    # 2 the pull request. Anything else is not that commit.
                    parents = git("rev-list", "--parents", "-n", "1", args.head).split()[1:]
                    if len(parents) != 2:
                        raise RuntimeError(f"{args.head} has {len(parents)} parents, not a merge commit")
                files = changed_files(args.base, args.head)
            except Exception as e:  # noqa: BLE001
                reason = f"could not compute the diff ({e})"

    write(plan(ws, files, reason), args.out)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # noqa: BLE001 - never fail the job; test everything
        print(f"::warning::select-tests.py failed ({e}); running the full suite.")
        out = None
        argv = sys.argv[1:]
        if "--out" in argv and argv.index("--out") + 1 < len(argv):
            out = Path(argv[argv.index("--out") + 1])
        lines = ["mode=full", "docker=true", "mongo=true", "persistence=true", "tests=", "skips="]
        if out:
            out.write_text("\n".join(lines) + "\n", encoding="utf-8")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as f:
                f.write("\n".join(lines[:4]) + "\n")
        sys.exit(0)
