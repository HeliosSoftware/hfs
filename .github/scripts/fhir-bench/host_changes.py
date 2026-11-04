"""Changes of the benchmark Docker host that make older runs not strictly comparable.

Permanent on purpose (unlike legacy_points.py): add one entry whenever the shared Docker
host or its resources change. history.py and report_html.py use it to say, under the
tables and charts, that runs before `at` were measured on different hardware.
"""

HOST_CHANGES = [
    {
        "at": "2026-10-28T14:40:00Z",
        "approximate": False,
        "note": "the Docker host CPUs were raised from 4 to 8",
    },
]


def _ts(value):
    """Comparable 'YYYY-MM-DDTHH:MM:SS' prefix of an ISO timestamp or a bare date."""
    v = (value or "").strip().rstrip("Z")
    return (v + "T00:00:00")[:19] if len(v) == 10 else v[:19]


def changes_between(first, last):
    """Host changes that fall inside the span [first, last] of run timestamps (ISO or date)."""
    a, b = _ts(first), _ts(last)
    if not a or not b:
        return []
    return [c for c in HOST_CHANGES if a < _ts(c["at"]) <= b]


def before_change(ts, change):
    """True when the run timestamp `ts` is earlier than the host change."""
    t = _ts(ts)
    return bool(t) and t < _ts(change["at"])


def legend(change):
    """One legend sentence, the same text in the Markdown history and the HTML report."""
    when = change["at"][:16].replace("T", " ") + " UTC" + (" (approximate)" if change.get("approximate") else "")
    return (f"Runs before {when} were measured on an older Docker host: {change['note']}. "
            "Part of a difference across that date can come from the hardware, not the code.")
