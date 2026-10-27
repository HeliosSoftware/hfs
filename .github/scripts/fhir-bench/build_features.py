#!/usr/bin/env python3
"""Derive the `cargo build --features` list from the resolved benchmark matrix.

Usage: build_features.py '<matrix json>'   (or the JSON on stdin)

The matrix is resolve-matrix.sh's output, {"include":[{"backend":"postgres-elasticsearch",...}]}.
Mapping: sqlite -> sqlite; postgres -> postgres; mongodb -> mongodb;
<x>-elasticsearch -> x plus elasticsearch. FHIR R4 is always on (no other
FHIR version is ever compiled). Output order is fixed so the same leg set
always yields the same string (it is also part of the build cache key).
"""
import json
import sys

PRIMARIES = ("sqlite", "postgres", "mongodb")
ORDER = ("R4", "sqlite", "postgres", "mongodb", "elasticsearch")


def features_for(matrix):
    feats = {"R4"}
    legs = matrix.get("include", [])
    if not legs:
        raise ValueError("matrix has no legs")
    for leg in legs:
        name = leg["backend"]
        base, sep, es = name.partition("-")
        if base not in PRIMARIES or (sep and es != "elasticsearch"):
            raise ValueError(f"unknown backend leg {name!r}")
        feats.add(base)
        if sep:
            feats.add("elasticsearch")
    return ",".join(f for f in ORDER if f in feats)


def main(argv):
    raw = argv[1] if len(argv) > 1 else sys.stdin.read()
    try:
        print(features_for(json.loads(raw)))
    except (ValueError, KeyError, TypeError) as e:
        print(f"::error::build_features: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
