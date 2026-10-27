#!/usr/bin/env bash
# Guard for issue #200.
#
# Same-URL precedence (which of several rows sharing a canonical URL wins) must
# be expressed ONCE, in `backends::cs_precedence_order_by` / `vs_precedence_order_by`.
# The bug this guards against was caused by that rule being re-implemented ad hoc
# in ~40 backend queries that drifted apart, so that `$validate-code` resolved one
# row while `$lookup` read another.
#
# Fails if any query orders `code_systems` / `value_sets` by a hand-rolled version
# sort instead of the shared helper.
set -euo pipefail
cd "$(dirname "$0")/.."

# A bare `COALESCE(<alias>.version, '') DESC` sort is the signature of the old,
# divergent rule. Match ANY table alias (`version`, `s.version`, `cs.version`,
# `code_systems.version`, …) so a query cannot dodge the guard by renaming its
# alias — the earlier three-literal form let exactly that slip through.
#
# The exclusion pattern tolerates a repeated slash: BSD grep (the macOS lint
# runners) prints `src//backends/mod.rs` for a `src/` root, so a literal
# `src/backends/mod.rs` filter missed it and the guard failed on macOS only.
#
# `backends/mod.rs` is the one sanctioned home of the ordering: the shared
# helpers there emit `COALESCE({alias}.version, '') DESC` with a `{alias}`
# placeholder, which does not match the concrete-alias pattern anyway, but it is
# excluded explicitly so the guard stays correct even if a helper is later
# rewritten to use a literal alias.
if hits=$(grep -rnE --include=*.rs \
        "COALESCE\(([A-Za-z_][A-Za-z0-9_]*\.)?version, ''\) DESC" \
        src/ 2>/dev/null | grep -vE 'src/+backends/mod\.rs'); then
    echo "ERROR: hand-rolled same-URL ordering found." >&2
    echo "Use crate::backends::cs_precedence_order_by() / vs_precedence_order_by() instead." >&2
    echo "See crates/hts/docs/ig-publisher-compatibility.md §2a (issue #200)." >&2
    echo "$hits" >&2
    exit 1
fi

echo "OK: same-URL precedence is centralized."
