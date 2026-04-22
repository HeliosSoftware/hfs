#!/usr/bin/env python3
"""Rewrite generated validate_*_binding(validator, path, vs, ...) calls to BindingCheckContext.

Matches fhir_validation::r{4|4b|5|6}:: — earlier migrations incorrectly used r[46]b? which skipped r5.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

# sync: let child_issues = fhir_validation::rX::validate_...(validator, ...
PREFIX_RE = re.compile(
    r"^(?P<indent>\s*)let child_issues = (?P<prefix>fhir_validation::r(?:4b|4|5|6)::validate_[a-z_]+(?:_async)?)\(validator,\s*"
)

def transform_line(line: str) -> str | None:
    if "BindingCheckContext" in line:
        return None
    m = PREFIX_RE.match(line)
    if not m:
        return None

    indent = m.group("indent")
    prefix = m.group("prefix")
    rest = line[m.end() :].rstrip()

    if rest.endswith(").await;"):
        inner = rest[: -len(").await;")]
        is_async_call = True
    elif rest.endswith(");"):
        inner = rest[: -len(");")]
        is_async_call = False
    else:
        return None

    if ", terminology" not in inner:
        return None
    inner2 = inner.rsplit(", terminology", 1)[0]

    m2 = re.match(
        r'^"(?P<path>[^\"]*)",\s*"(?P<vs>[^\"]*)",\s*(?P<strength>fhir_validation_types::BindingStrength::\w+),\s*(?P<payload>.+)$',
        inner2.strip(),
    )
    if not m2:
        return None

    fhir_path = m2.group("path")
    value_set = m2.group("vs")
    strength = m2.group("strength")
    payload_and_closure = m2.group("payload").strip()

    fn_async = prefix.endswith("_async")
    if fn_async != is_async_call:
        return None

    ctx_ty = "BindingCheckContextAsync" if fn_async else "BindingCheckContextSync"
    await_suffix = ".await" if fn_async else ""

    return (
        f"{indent}let binding_ctx = fhir_validation::binding::common::{ctx_ty}::new(validator, "
        f'"{fhir_path}", "{value_set}", {strength}, terminology);\n'
        f"{indent}let child_issues = {prefix}(&binding_ctx, {payload_and_closure}){await_suffix};"
    )


def process_file(path: Path) -> bool:
    text = path.read_text(encoding="utf-8")
    out_lines: list[str] = []
    changed = False
    for line in text.splitlines(True):
        nl = line if line.endswith("\n") else line + "\n"
        bare = nl.rstrip("\n")
        new_block = transform_line(bare)
        if new_block is not None:
            out_lines.append(new_block + "\n")
            changed = True
        else:
            out_lines.append(nl)

    if changed:
        path.write_text("".join(out_lines), encoding="utf-8")
    return changed


def main() -> None:
    roots = [Path(p) for p in sys.argv[1:]] or [
        Path(__file__).resolve().parents[1] / "generated" / "r5" / "parts"
    ]
    n = 0
    for root in roots:
        if root.is_file() and root.suffix == ".rs":
            if process_file(root):
                n += 1
                print(f"updated {root}")
        elif root.is_dir():
            for f in sorted(root.glob("*.rs")):
                if process_file(f):
                    n += 1
                    print(f"updated {f}")
    print(f"done, {n} file(s) changed")


if __name__ == "__main__":
    main()
