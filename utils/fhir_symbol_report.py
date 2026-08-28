#!/usr/bin/env python3
"""Attribute a test binary's symbol bytes to the generated FHIR model code.

This is the measurement behind #508 / #509 / #510. Those issues quote symbol
sizes per category and, more importantly, a *copies* column: how many times the
same generated body was emitted, once per `(FHIR type, Deserializer)` or
`(FHIR type, Serializer)` pair. A body at 1.0 copies is at its floor; anything
above that is monomorphization tax that can be engineered away.

Usage:

    export CARGO_PROFILE_DEV_DEBUG=0
    cargo test -p helios-rest --lib --all-features --no-run
    BIN=$(ls target/debug/deps/helios_rest-* | grep -v '\\.d$' | grep -v rcgu | head -1)
    nm -S -C "$BIN" > after.nm
    utils/fhir_symbol_report.py after.nm            # one binary
    utils/fhir_symbol_report.py before.nm after.nm  # before/after with deltas

Note that `nm -C` demangles, which collapses distinct monomorphizations of the
same function into one name — that is exactly what makes the copies column
meaningful. It also collapses items declared inside an anonymous `const _: () =
{ … }` block, so for those the copies column is not a duplication signal; pass
the mangled dump (`nm -S`) if you need to tell them apart.
"""

import collections
import re
import sys

LINE = re.compile(r"^([0-9a-fA-F]+)\s+([0-9a-fA-F]+)\s+(\S)\s+(.*)$")


def bucket(name: str) -> str | None:
    """Classify a demangled symbol, or None if it is not FHIR model code."""
    if "helios_fhir::" not in name:
        return None
    # serde's own `#[derive(Deserialize)]` on the generated `Temp*` structs.
    if re.search(r"\bTemp[A-Z]", name):
        return "temp-struct Deserialize"
    # The `field` / `_field` reunification hoisted out of the derive by #509.
    if "__from_temp" in name:
        return "__from_temp"
    if "EnumVisitor" in name:
        return "choice EnumVisitor"
    # `__FhirSerShim` is the erasure shim the derive routes `Serialize` through.
    if "ser::Serialize>::serialize" in name or "__FhirSerShim" in name:
        return "Serialize"
    if re.search(r"de::Deserialize>::deserialize$", name) or "__fhir_erase_ser" in name:
        return "deserialize/erase wrapper"
    if "IntoEvaluationResult" in name or "to_evaluation_result" in name:
        return "FHIRPath"
    return "fhir other"


def collect(path: str) -> tuple[dict[str, dict[str, list[int]]], int]:
    data: dict[str, dict[str, list[int]]] = collections.defaultdict(
        lambda: collections.defaultdict(lambda: [0, 0])
    )
    total = 0
    with open(path) as handle:
        for line in handle:
            match = LINE.match(line.rstrip("\n"))
            if not match:
                continue
            size = int(match.group(2), 16)
            name = match.group(4)
            total += size
            group = bucket(name)
            if group is None:
                continue
            entry = data[group][name]
            entry[0] += 1
            entry[1] += size
    return data, total


def report(path: str, label: str) -> dict[str, int]:
    data, total = collect(path)
    print(f"--- {label}  (all symbol bytes: {total / 1e6:.1f} MB) ---")
    print(f"{'bucket':<27}{'bytes':>11}{'occurrences':>13}{'names':>8}{'copies':>8}")
    sizes = {}
    for group in sorted(data, key=lambda k: -sum(v[1] for v in data[k].values())):
        entries = data[group]
        byt = sum(v[1] for v in entries.values())
        occ = sum(v[0] for v in entries.values())
        sizes[group] = byt
        print(
            f"{group:<27}{byt / 1e6:>8.1f} MB{occ:>13}{len(entries):>8}"
            f"{occ / len(entries):>8.1f}"
        )
    print(f"{'FHIR TOTAL':<27}{sum(sizes.values()) / 1e6:>8.1f} MB")
    print()
    return sizes


def main() -> None:
    args = sys.argv[1:]
    if not args or len(args) > 2:
        sys.exit(__doc__)

    if len(args) == 1:
        report(args[0], args[0])
        return

    before = report(args[0], f"before  ({args[0]})")
    after = report(args[1], f"after   ({args[1]})")

    print(f"{'bucket':<27}{'before':>11}{'after':>11}{'delta':>11}")
    for group in sorted(
        set(before) | set(after), key=lambda k: after.get(k, 0) - before.get(k, 0)
    ):
        b, a = before.get(group, 0), after.get(group, 0)
        print(f"{group:<27}{b / 1e6:>8.1f} MB{a / 1e6:>8.1f} MB{(a - b) / 1e6:>+8.1f} MB")
    b, a = sum(before.values()), sum(after.values())
    print(f"{'TOTAL':<27}{b / 1e6:>8.1f} MB{a / 1e6:>8.1f} MB{(a - b) / 1e6:>+8.1f} MB")


if __name__ == "__main__":
    main()
