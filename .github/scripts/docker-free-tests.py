#!/usr/bin/env python3
"""Print the `cargo test` arguments that select the workspace's tests that do
not need Docker.

    python .github/scripts/docker-free-tests.py [--list]

Used where `cargo test` runs without a Docker daemon: the Windows nightly on
GitHub's `windows-latest`, which cannot reach the Docker host the Linux agents
use for testcontainers (`DOCKER_HOST`). Prints one argument per line:

    --lib / --bins          every crate's unit tests and binaries' tests
    --test <name>           each integration test target that does not start a
                            container
    --                      then, for the test harness:
    --skip <module path>    each library module that starts containers in its
                            unit tests

A target "starts containers" if any file it compiles mentions testcontainers.
That is found by following `mod` declarations (and `#[path]`) from the target's
root file, so a test that reaches containers only through `mod common;` counts.
The check is deliberately coarse: it skips whole targets and whole modules, so
it may drop some tests that would have run, never keep one that needs Docker.

`--list` prints the classification instead, for review.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

DOCKER = re.compile(r"\btestcontainers\b|\bGenericImage\b|\bContainerAsync\b")
# `mod name;` with an optional `#[path = "..."]` on the line(s) before it.
# Inline `mod name { ... }` has a brace, not a semicolon, and needs no file.
MOD_DECL = re.compile(
    r'(?:#\[path\s*=\s*"(?P<path>[^"]+)"\]\s*)?'
    r"(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*;"
)
# A `#[path]` only on an inline `mod name {` (rare) changes where nested
# `mod x;` resolve; not handled, as nothing in the workspace does it.


def strip_comments(src: str) -> str:
    src = re.sub(r"/\*.*?\*/", "", src, flags=re.S)
    return re.sub(r"//[^\n]*", "", src)


def module_files(root: Path) -> set[Path]:
    """Every file compiled into the module tree rooted at `root`."""
    seen: set[Path] = set()
    stack = [root]
    while stack:
        f = stack.pop()
        f = f.resolve()
        if f in seen or not f.is_file():
            continue
        seen.add(f)
        text = strip_comments(f.read_text(encoding="utf-8", errors="replace"))
        # Children of `foo.rs` live in `foo/`; of `mod.rs`, `lib.rs`,
        # `main.rs` and a test target's root, beside it.
        is_dir_owner = f.name in ("mod.rs", "lib.rs", "main.rs") or f == root.resolve()
        base = f.parent if is_dir_owner else f.parent / f.stem
        for m in MOD_DECL.finditer(text):
            if m.group("path"):
                stack.append(f.parent / m.group("path"))
            else:
                name = m.group("name")
                stack.append(base / f"{name}.rs")
                stack.append(base / name / "mod.rs")
    return seen


def needs_docker(files: set[Path]) -> bool:
    return any(DOCKER.search(p.read_text(encoding="utf-8", errors="replace")) for p in files)


def lib_skips(src_root: Path, files: set[Path]) -> list[str]:
    """Module paths, relative to the crate root, of library files that start
    containers. The crate root itself is never one: a hit there is the
    test-only exit hook (`container_cleanup`), which starts nothing."""
    src_dir = src_root.parent
    out = []
    for f in sorted(files):
        if f == src_root.resolve() or not DOCKER.search(f.read_text(encoding="utf-8", errors="replace")):
            continue
        try:
            rel = f.relative_to(src_dir.resolve())
        except ValueError:
            continue  # a `#[path]` outside src/, e.g. tests/common/...
        parts = list(rel.with_suffix("").parts)
        if parts[-1] == "mod":
            parts.pop()
        out.append("::".join(parts) + "::")
    # Drop paths already covered by a shorter one.
    out.sort(key=len)
    kept: list[str] = []
    for p in out:
        if not any(p.startswith(k) for k in kept):
            kept.append(p)
    return kept


def main() -> int:
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    members = set(meta["workspace_members"])
    by_name: dict[str, list[bool]] = {}
    skips: list[str] = []
    report = []
    for pkg in meta["packages"]:
        if pkg["id"] not in members:
            continue
        for t in pkg["targets"]:
            root = Path(t["src_path"])
            if "test" in t["kind"]:
                docker = needs_docker(module_files(root))
                by_name.setdefault(t["name"], []).append(docker)
                report.append(f"{'docker' if docker else 'run   '} {pkg['name']} --test {t['name']}")
            elif "lib" in t["kind"] or "cdylib" in t["kind"] or "rlib" in t["kind"]:
                for s in lib_skips(root, module_files(root)):
                    skips.append(s)
                    report.append(f"skip   {pkg['name']} lib {s}")
    # `--test <name>` selects that name in every package that has it, so a
    # name is run only if no package's target of that name needs Docker.
    run = sorted(n for n, flags in by_name.items() if not any(flags))

    if "--list" in sys.argv[1:]:
        print("\n".join(sorted(report)))
        print(f"\n{len(run)} integration targets run, "
              f"{sum(1 for f in by_name.values() if any(f))} need Docker, "
              f"{len(skips)} library modules skipped", file=sys.stderr)
        return 0

    args = ["--lib", "--bins"]
    for n in run:
        args += ["--test", n]
    args.append("--")
    for s in sorted(set(skips)):
        args += ["--skip", s]
    print("\n".join(args))
    return 0


if __name__ == "__main__":
    sys.exit(main())
