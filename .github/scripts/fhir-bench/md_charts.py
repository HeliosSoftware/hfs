#!/usr/bin/env python3
"""Collapsed Mermaid charts for the GitHub job summary of the FHIR benchmark.

    python3 md_charts.py compare --in comparison.md --layout sidebyside
    python3 md_charts.py leg --box leg-summary.md --leg sqlite --run-dir bench-results --layout sidebyside

Both print a `### Charts` Markdown section on stdout, meant to be APPENDED after the
unchanged report text (`cat summary.md; python3 md_charts.py ...`). Nothing of the
original is ever rewritten. The charts read their numbers from the tables of the text
they sit under, so a chart cannot disagree with its table; the run-phase timeline and
the bulk-import phases of a leg also read the leg's result files.

Always exits 0 and prints nothing (or one short note) when something cannot be parsed.
Standard library only. GitHub renders Mermaid (xychart-beta, gantt) in job summaries;
inline SVG, data: images and colour styles are stripped, so colours go through the
`%%{init}%%` theme variables.
"""
import argparse
import json
import math
import os
import re
import sys
import zlib

BLUE = "#2f81f7"
RAMP_P99, RAMP_P95, RAMP_P50 = "#54aeff", "#2f81f7", "#0969da"  # p99 drawn at the back
GANTT_BAR = "#0969da"
GANTT_BAR_2 = "#54aeff"
GRID = "#8c959f"
XY_W, XY_H = 540, 260
XY_W_CELL = 440  # two cells must fit GitHub's ~1012 px content width
SPREAD_NOTE = 50.0
MAX_OUT_BYTES = 600_000  # a step summary is capped at 1 MiB; the report text comes first

# Leg colours: the six known legs keep the colours of the approved prototype's HTML report
# (mid tones readable on both GitHub themes); any other leg name gets one of EXTRA, picked
# by a hash of its name so it is stable, and moved on if another leg of the chart has it.
LEG_COLOURS = {
    "postgres": "#236bcf", "sqlite": "#008622", "mongodb": "#9348b1",
    "postgres-elasticsearch": "#4a91f8", "sqlite-elasticsearch": "#3fac4a",
    "mongodb-elasticsearch": "#b96dd8",
}
EXTRA = ["#db6d28", "#bf8700", "#cf222e", "#1b9aaa", "#e5609a", "#6e7781", "#8a5a00", "#57ab5a",
         "#c2410c", "#0e7490"]

MARKERS = "†‡⚠"


# ── table parsing ───────────────────────────────────────────────────────────
def cells(line):
    return [c.strip() for c in line.strip().strip("|").split("|")]


def find_table(lines, heading, lo=0):
    """(header, rows, last_row_index) of the first table after a line starting with `heading`."""
    for i in range(lo, len(lines)):
        if not lines[i].startswith(heading):
            continue
        j = i + 1
        while j < len(lines) and not lines[j].startswith("|"):
            if lines[j].startswith("#"):
                return None
            j += 1
        if j + 1 >= len(lines) or not re.match(r"^\|[-:| ]+\|$", lines[j + 1].strip()):
            return None
        head = cells(lines[j])
        rows, k = [], j + 2
        while k < len(lines) and lines[k].startswith("|"):
            row = cells(lines[k])
            rows.append(row + [""] * (len(head) - len(row)))
            k += 1
        return head, rows, k - 1
    return None


def num(text):
    """First number of a cell (markers, bold and thousands commas ignored), or None for n/a."""
    t = text.replace("**", "").lstrip(" " + MARKERS)
    m = re.match(r"^(-?\d[\d,]*(?:\.\d+)?|-?\.\d+)", t)
    return float(m.group(1).replace(",", "")) if m else None


def plain(text):
    """A cell as text safe inside a quoted Mermaid string."""
    t = text.replace("**", "").replace("`", "").replace('"', "'")
    t = t.replace("#", "").replace(";", ",").replace("[", "(").replace("]", ")").replace("\\", "")
    return re.sub(r"\s+", " ", t).strip()


def title_safe(text):
    return plain(text).replace("$", "").replace("|", "/").replace("*", "").replace("_", " ")


def short_name(label):
    """'Batch/Transaction Import (`/ui/batch`: ...) (resources/s)' -> ('Batch/Transaction Import', '(resources/s)')."""
    label = label.replace("**", "")
    unit = re.findall(r"\(([^()]*)\)", label)
    name = re.split(r"[,(]", label, maxsplit=1)[0].strip()
    if name.startswith("↳"):  # phase rows of the bulk job: name them in full
        name = "Bulk Import " + name.lstrip("↳ ").strip()
    name = title_safe(name)
    u = ""
    if unit:
        # last parenthesised group that looks like a unit ("per Bundle" is a latency note, kept as is)
        u = "(%s)" % title_safe(unit[-1])
    return name, u


def canon_leg(label):
    t = label.replace("`", "").strip().lower()
    if t.endswith("+es"):
        t = t[:-3] + "-elasticsearch"
    return t


def leg_colours(names):
    """{name: colour}, distinct within `names`, stable per name for the known legs."""
    out, used = {}, set()
    for n in names:
        c = LEG_COLOURS.get(canon_leg(n))
        if c:
            out[n] = c
            used.add(c)
    for n in names:
        if n in out:
            continue
        start = zlib.crc32(canon_leg(n).encode("utf-8")) % len(EXTRA)
        pick = None
        for k in range(len(EXTRA)):
            c = EXTRA[(start + k) % len(EXTRA)]
            if c not in used:
                pick = c
                break
        if pick is None:  # more legs than colours: cycle
            pick = EXTRA[(start + len(used)) % len(EXTRA)]
        out[n] = pick
        used.add(pick)
    return out


# ── Mermaid builders ────────────────────────────────────────────────────────
def nice_top(v):
    if v <= 0:
        return 1
    e = 10 ** math.floor(math.log10(v * 1.08))
    for f in (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10):
        if f * e >= v * 1.08:
            return int(f * e) if f * e >= 1 else f * e
    return 10 * e


def mnum(v):
    return ("%.2f" % v).rstrip("0").rstrip(".")


def xy(cats, series, palette, y_label, horizontal=False, width=XY_W, height=XY_H, value_ticks=True):
    """series: list of ('bar'|'line', values); None counts as 0."""
    top = nice_top(max([v or 0 for _, vals in series for v in vals] + [0]))
    cfg = {"width": width, "height": height, "useMaxWidth": False}
    if not value_ticks:
        cfg["yAxis"] = {"showLabel": False, "showTick": False}
    init = {"xyChart": cfg,
            "themeVariables": {"xyChart": {"plotColorPalette": ", ".join(palette),
                                           "backgroundColor": "transparent"}}}
    out = ["```mermaid", "%%{init: " + json.dumps(init) + "}%%",
           "xychart-beta" + (" horizontal" if horizontal else ""),
           "    x-axis [" + ", ".join('"%s"' % c for c in cats) + "]",
           '    y-axis "%s" 0 --> %s' % (plain(y_label), top)]
    for kind, vals in series:
        out.append("    %s [%s]" % (kind, ", ".join(mnum(v or 0) for v in vals)))
    out.append("```")
    return out


def hms(s):
    s = int(s) % 86400
    return "%02d:%02d:%02d" % (s // 3600, s % 3600 // 60, s % 60)


def gantt_init(left):
    return {"gantt": {"useMaxWidth": False, "barHeight": 16, "barGap": 4,
                      "fontSize": 11, "sectionFontSize": 11, "leftPadding": left},
            "themeVariables": {"taskBkgColor": GANTT_BAR, "taskBorderColor": GANTT_BAR,
                               "taskTextColor": "#ffffff", "taskTextLightColor": "#ffffff",
                               "doneTaskBkgColor": GANTT_BAR_2, "doneTaskBorderColor": GANTT_BAR_2,
                               "taskTextDarkColor": "#1f2328", "gridColor": GRID,
                               "sectionBkgColor": "transparent", "altSectionBkgColor": "transparent",
                               "sectionBkgColor2": "transparent"},
            "themeCSS": ".grid .tick line{stroke:%s;stroke-opacity:.6}" % GRID}


def gtext(s):
    """Task and section names: no colon, comma, semicolon or hash."""
    return re.sub(r"[:,;#]", " ", plain(s)).strip() or "leg"


def bulk_gantt(rows):
    """rows: (leg, ingest_s, rebuild_s or None). Elapsed time since kick-off. A leg without a
    rebuild (indexing ran during ingest) gets the ingest bar only."""
    out = ["```mermaid", "%%{init: " + json.dumps(gantt_init(150)) + "}%%", "gantt",
           "    dateFormat YYYY-MM-DD HH:mm:ss", "    axisFormat %H:%M", "    todayMarker off"]
    for n, (leg, a, b) in enumerate(rows):
        out.append("    section %s" % gtext(leg))
        out.append("    ingest %d s :i%d, 2000-01-01 00:00:00, 2000-01-01 %s" % (round(a), n, hms(round(a))))
        if b and b > 0:
            out.append("    rebuild %d s :done, x%d, after i%d, %ds" % (round(b), n, n, max(1, round(b))))
    out.append("```")
    return out


def phase_gantt(ph):
    out = ["```mermaid", "%%{init: " + json.dumps(gantt_init(60)) + "}%%", "gantt",
           "    dateFormat YYYY-MM-DD HH:mm:ss", "    axisFormat %H:%M", "    todayMarker off",
           "    section Suites"]
    for i, (suite, a, b) in enumerate(ph):
        da, db = "2000-01-%02d" % (1 + a // 86400), "2000-01-%02d" % (1 + b // 86400)
        out.append("    %s :t%d, %s %s, %s %s" % (gtext(suite), i, da, hms(a), db, hms(b)))
    out.append("```")
    return out


def details(summary, body):
    return ["", "<details><summary>Chart: %s</summary>" % summary, ""] + body + ["", "</details>"]


def titled(title, how, chart):
    return ["**%s**" % title, "", how, ""] + chart


def grid(blocks, layout):
    out = []
    if layout == "sidebyside":
        out.append("<table>")
        for i in range(0, len(blocks), 2):
            out.append("<tr>")
            for b in blocks[i:i + 2]:
                out += ['<td valign="top">', ""] + b + ["", "</td>"]
            out.append("</tr>")
        out.append("</table>")
    else:
        for i, b in enumerate(blocks):
            out += ([""] if i else []) + b
    return out


def spread(vals):
    pos = [v for v in vals if v and v > 0]
    return max(pos) / min(pos) if len(pos) > 1 else 1.0


def section(blocks):
    """The `### Charts` section from [(summary, body_lines, table_name)]."""
    body = []
    for summary, lines, table in blocks:
        lines = [l.replace("values in the table above", "values in the %s table above" % table)
                 for l in lines]
        body += details(summary, lines)
    return ["", "### Charts"] + body if body else []


# ── bulk import rows ────────────────────────────────────────────────────────
def _secs_in_parens(text):
    m = re.search(r"\(\s*([\d,]+(?:\.\d+)?)\s*s\s*\)", text)
    return float(m.group(1).replace(",", "")) if m else None


def _rebuild_from_text(text):
    """Seconds of a REAL deferred rebuild in an indexing cell, else None.
    `210 s (= ingest)` and `inline (during ingest)` mean: indexed during ingest, no rebuild."""
    t = text.replace("**", "").replace("`", "")
    inline = re.match(r"^\s*[" + MARKERS + r"]*\s*inline\b", t, re.I)
    if "= ingest" in t or (inline and "rebuild" not in t):
        return None
    m = re.search(r"\+\s*([\d,]+(?:\.\d+)?)\s*s\s+rebuild", t)
    if m:
        return float(m.group(1).replace(",", ""))
    if re.match(r"^\s*[" + MARKERS + r"]*\s*\d", t):
        v = num(t)
        return v if v and v > 0 else None
    return None


def bulk_phases_from_throughput(head, rows):
    """[(leg, ingest_s, rebuild_s|None)] from the `↳ ingest` / `↳ search indexing` rows."""
    ing = idx = None
    for row in rows:
        lab = row[0].replace("**", "")
        if not lab.startswith("↳"):
            continue
        low = lab.lower()
        if "ingest" in low and "indexing" not in low:
            ing = row
        elif "index" in low:
            idx = row
    if not ing:
        return []
    out = []
    for i, leg in enumerate(head[1:], start=1):
        a = _secs_in_parens(ing[i])
        if a is None:
            continue
        b = _rebuild_from_text(idx[i]) if idx else None
        out.append((plain(leg), a, b))
    return out


def bulk_phases_from_table(head, rows):
    col = {h: i for i, h in enumerate(head)}
    if "Ingest" not in col or "Index" not in col:
        return []
    out = []
    for row in rows:
        a = num(row[col["Ingest"]])
        if a is None:
            continue
        out.append((plain(row[0]), a, _rebuild_from_text(row[col["Index"]])))
    return out


def bulk_gantt_block(phases):
    any_rebuild = any(b for _, _, b in phases)
    how = ("Per leg, ingest until Completed (dark)"
           + (", then the deferred search-index rebuild (light) on the legs that run one; the others index during ingest"
              if any_rebuild else "; indexing runs during ingest on these legs, so each bar is the ingest alone")
           + ", on elapsed time since kick-off; shorter is better; values in the table above.")
    title = ("Bulk Import: ingest until Completed, then search-index rebuild" if any_rebuild
             else "Bulk Import: ingest until Completed")
    return titled(title, how, bulk_gantt(phases))


# ── comparison ──────────────────────────────────────────────────────────────
def compare_section(lines, layout):
    w = XY_W_CELL if layout == "sidebyside" else XY_W
    blocks = []
    for heading, kind, tname in (("### Throughput", "thr", "Throughput"),
                                 ("### p95 latency", "p95", "p95 latency")):
        r = find_table(lines, heading)
        if not r:
            continue
        head, rows, _ = r
        legs = [plain(h) for h in head[1:]]
        colours = leg_colours(legs)
        charts = []
        for row in rows:
            vals = [num(c) for c in row[1:]]
            if not vals or all(v is None for v in vals):
                continue  # e.g. Bulk Import p95: n/a (one async job)
            name, unit = short_name(row[0])
            if kind == "thr" and not unit:
                unit = "(requests/s)"
            inner = unit.strip("()")
            seconds_only = "seconds" in inner and "/s" not in inner
            lower = kind == "p95" or seconds_only
            order = sorted(range(len(legs)), key=lambda i: -(vals[i] or 0))
            cats = ["%s · %s" % (legs[i], plain(row[1 + i])) for i in order]
            ovals = [vals[i] for i in order]
            series = []
            for pos, i in enumerate(order):
                series.append(("bar", [(vals[i] or 0) if p == pos else 0 for p in range(len(order))]))
            pal = [colours[legs[i]] for i in order]
            how = ("One bar per leg, largest first, %s is better; values in the table above, repeated "
                   "in each label (with its † ‡ ⚠ markers)." % ("shorter" if lower else "longer"))
            if spread(ovals) > SPREAD_NOTE:
                how += (" Values span %dx (no log axis in Mermaid): short bars are read from their "
                        "label." % spread(ovals))
            title = "%s %s" % (name, "p95 (ms)" if kind == "p95" else unit)
            ylab = "ms" if kind == "p95" else inner.split(",")[0]
            charts.append(titled(title, how, xy(cats, series, pal, ylab, horizontal=True, width=w,
                                                height=max(220, 100 + 20 * len(legs)), value_ticks=False)))
        if charts:
            what = ("throughput by leg, one chart per row" if kind == "thr"
                    else "p95 latency by leg, one chart per row")
            blocks.append((what, grid(charts, layout), tname))
    phases, tname = [], "Throughput"
    r = find_table(lines, "### Throughput")
    if r:
        phases = bulk_phases_from_throughput(r[0], r[1])
    if not phases:
        r = find_table(lines, "### Bulk import")
        if r:
            phases, tname = bulk_phases_from_table(r[0], r[1]), "Bulk import"
    if phases:
        blocks.append(("Bulk Import phases by leg", bulk_gantt_block(phases), tname))
    return section(blocks)


# ── one leg ─────────────────────────────────────────────────────────────────
def read_kv(path, sep="="):
    kv = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if sep in line:
                    k, v = line.rstrip("\n").split(sep, 1)
                    kv[k.strip()] = v.strip()
    except OSError:
        pass
    return kv


def result_dir(run_dir, leg):
    for d in (run_dir, os.path.join(run_dir, leg), os.path.join(run_dir, leg, leg)):
        if d and os.path.isdir(d) and (os.path.exists(os.path.join(d, "host-contention.txt"))
                                       or os.path.exists(os.path.join(d, "bulk-import.txt"))):
            return d
    return run_dir if run_dir and os.path.isdir(run_dir) else None


def phases(d):
    if not d:
        return []
    try:
        with open(os.path.join(d, "host-contention.txt"), encoding="utf-8", errors="replace") as fh:
            text = fh.read()
    except OSError:
        return []
    start, out, day, last = {}, [], 0, None
    for m in re.finditer(r"^(\d\d):(\d\d):(\d\d)Z suite=(\S+) phase=(start|end)", text, re.M):
        s = int(m.group(1)) * 3600 + int(m.group(2)) * 60 + int(m.group(3))
        if last is not None and s + day < last:
            day += 86400
        s += day
        last = s
        if m.group(5) == "start":
            start[m.group(4)] = s
        elif m.group(4) in start:
            out.append((m.group(4), start.pop(m.group(4)), s))
    return out


def leg_bulk_phase(d, leg):
    kv = read_kv(os.path.join(d, "bulk-import.txt")) if d else {}
    try:
        a = float(kv["ingest_seconds"])
    except (KeyError, ValueError):
        return []
    b = None
    if kv.get("index_status") != "inline":
        try:
            b = float(kv.get("index_seconds", ""))
        except ValueError:
            b = None
    return [(leg, a, b if b and b > 0 else None)]


def leg_section(lines, leg, run_dir, layout):
    colour = leg_colours([leg])[leg]
    d = result_dir(run_dir, leg) if run_dir else None
    start = 0
    for i, l in enumerate(lines):
        if l.startswith("## FHIR Benchmark") and "(%s)" % leg in l:
            start = i
            break
    r = find_table(lines, "### Results", start) or find_table(lines, "### Results")
    blocks = []
    ph = phases(d)
    if ph:
        how = ("Each bar is one suite of this leg, from its start to its end (UTC, from "
               "host-contention.txt); gaps are setup between suites.")
        blocks.append(("%s run phases (UTC)" % plain(leg),
                       titled("%s: run phases (UTC)" % title_safe(leg), how, phase_gantt(ph)), "Results"))
    bp = leg_bulk_phase(d, leg)
    if bp:
        blocks.append(("%s Bulk Import phases" % plain(leg), bulk_gantt_block(bp), "Results"))
    if r:
        head, rows, _ = r
        col = {h: i for i, h in enumerate(head)}
        if "Throughput" in col:
            th, imp, lat = [], [], []
            for row in rows:
                suite = row[0].replace("**", "")
                if suite.startswith("↳"):
                    continue
                v = num(row[col["Throughput"]])
                name, _u = short_name(suite)
                if suite.startswith("Batch/Transaction Import") or suite.startswith("Bulk Import"):
                    if v is not None:
                        imp.append(("batch import" if suite.startswith("Batch") else "bulk import", v))
                    continue
                if v is not None:
                    th.append((title_safe(plain(suite)) if "(" not in suite else name, v))
                p = [num(row[col[k]]) if k in col else None for k in ("p50 ms", "p95 ms", "p99 ms")]
                if None not in p:
                    lat.append((plain(suite), p))
            wl = XY_W_CELL if layout == "sidebyside" else XY_W
            pair = []
            if th:
                how = "One bar per k6 suite, higher is better; values in the Results table above."
                if spread([v for _, v in th]) > SPREAD_NOTE:
                    how = how[:-1] + "; values span more than %dx, so read short bars in the table." % SPREAD_NOTE
                pair.append(titled("%s: k6 suites throughput (requests/s)" % title_safe(leg), how,
                                   xy([n for n, _ in th], [("bar", [v for _, v in th])], [colour],
                                      "requests per second", width=wl)))
            if imp:
                how = ("One bar per way of loading the corpus, higher is better; values in the Results "
                       "table above (batch: committed Bundle entries per second of the k6 scenario; "
                       "bulk: resources until the search index covers them).")
                pair.append(titled("%s: Import throughput (resources/s)" % title_safe(leg), how,
                                   xy([n for n, _ in imp], [("bar", [v for _, v in imp])], [colour],
                                      "resources per second", width=wl)))
            body = grid(pair, layout) if pair else []
            if lat:
                how = ("Overlaid bars per suite, light to dark blue: p99 (back), p95, p50 (front); lower "
                       "is better; values in the Results table above (import, per Bundle, is left out).")
                if spread([x for _, p in lat for x in p]) > SPREAD_NOTE:
                    how += " Values span more than %dx; read small ones in the table." % SPREAD_NOTE
                cats = [n for n, _ in lat]
                series = [("bar", [p[2] for _, p in lat]), ("bar", [p[1] for _, p in lat]),
                          ("bar", [p[0] for _, p in lat])]
                body += ([""] if body else []) + titled(
                    "%s: latency per suite (ms)" % title_safe(leg), how,
                    xy(cats, series, [RAMP_P99, RAMP_P95, RAMP_P50], "ms"))
            if body:
                blocks.append(("%s throughput and latency" % plain(leg), body, "Results"))
    return section(blocks)


# ── CLI ─────────────────────────────────────────────────────────────────────
def read_lines(path):
    with open(path, encoding="utf-8", errors="replace") as fh:
        t = fh.read()
    return t[:-1].split("\n") if t.endswith("\n") else t.split("\n")


def render(lines):
    out = "\n".join(lines) + "\n" if lines else ""
    return out if len(out.encode("utf-8")) <= MAX_OUT_BYTES else ""


def main(argv=None):
    try:
        ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
        sub = ap.add_subparsers(dest="cmd")
        c = sub.add_parser("compare")
        c.add_argument("--in", dest="inp", required=True)
        c.add_argument("--layout", choices=("stacked", "sidebyside"), default="sidebyside")
        g = sub.add_parser("leg")
        g.add_argument("--box", required=True)
        g.add_argument("--leg", required=True)
        g.add_argument("--run-dir")
        g.add_argument("--layout", choices=("stacked", "sidebyside"), default="sidebyside")
        a = ap.parse_args(argv)
        if a.cmd == "compare":
            res = compare_section(read_lines(a.inp), a.layout)
        elif a.cmd == "leg":
            res = leg_section(read_lines(a.box), a.leg, a.run_dir, a.layout)
        else:
            return 0
        text = render(res)
        if text:
            sys.stdout.write(text)
    except SystemExit:
        pass
    except BaseException as e:  # never fail the job over charts
        try:
            print("<!-- charts skipped: %s -->" % type(e).__name__)
        except Exception:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
