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

# A bare version sort is the signature of the old, divergent rule.
if hits=$(grep -rn --include=*.rs \
        -e "ORDER BY COALESCE(version, '') DESC" \
        -e "ORDER BY COALESCE(s.version, '') DESC" \
        -e "ORDER BY COALESCE(cs.version, '') DESC" \
        src/ 2>/dev/null); then
    echo "ERROR: hand-rolled same-URL ordering found." >&2
    echo "Use crate::backends::cs_precedence_order_by() / vs_precedence_order_by() instead." >&2
    echo "See crates/hts/docs/ig-publisher-compatibility.md §2a (issue #200)." >&2
    echo "$hits" >&2
    exit 1
fi

echo "OK: same-URL precedence is centralized."
