#!/usr/bin/env python3
"""Reproduce the 30 fixed Synthea transaction Bundles (+ the 2 seed Bundles) of the Mongo bundle-import ABAB.

Selection rule (identical to the original harness; stdlib only):
  corpus  = the Synthea `bulk_1k` corpus used by .github/workflows/fhir-benchmark.yml:
            https://storage.googleapis.com/aidbox-public/synthea/performance/bulk_1k.tar.gz  (1,129 .json files)
  seeds   = files whose name starts with hospitalInformation / practitionerInformation (posted first, not measured)
  bundles = remaining *.json that parse as a FHIR Bundle, in plain sorted (code point) filename order
  slice10 = first 100 of them (the "10 % slice"); the ABAB uses the first 30 of the slice (BUNDLE_N=30)
The result is checked against expected/bundles30.sha256 (names + SHA-256 of each file).

Usage: python3 -I bin/prepare-dataset.py [--out data] [--corpus-dir DIR] [--n 30] [--no-verify]
"""
import argparse, hashlib, json, os, shutil, sys, tarfile, urllib.request

URL = "https://storage.googleapis.com/aidbox-public/synthea/performance/bulk_1k.tar.gz"
SEEDS = ("hospitalInformation", "practitionerInformation")
KIT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for c in iter(lambda: f.read(1 << 20), b""):
            h.update(c)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(KIT, "data"))
    ap.add_argument("--corpus-dir", help="already extracted bulk_1k directory (skips the download)")
    ap.add_argument("--n", type=int, default=30, help="how many patient Bundles to keep (the ABAB uses 30)")
    ap.add_argument("--no-verify", action="store_true", help="do not compare with expected/bundles30.sha256 (needed for --n other than 30)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    corpus = a.corpus_dir
    if not corpus:
        tgz = os.path.join(out, "bulk_1k.tar.gz")
        if not os.path.exists(tgz):
            print("downloading", URL, "(about 400 MB)")
            urllib.request.urlretrieve(URL, tgz + ".part")
            os.replace(tgz + ".part", tgz)
        print("tarball sha256 (informational, upstream may change it):", sha(tgz))
        corpus = os.path.join(out, "corpus")
        if not os.path.isdir(os.path.join(corpus, "bulk_1k")):
            shutil.rmtree(corpus, ignore_errors=True)
            os.makedirs(corpus)
            with tarfile.open(tgz) as t:
                try:
                    t.extractall(corpus, filter="data")
                except TypeError:  # python < 3.12
                    t.extractall(corpus)
        corpus = os.path.join(corpus, "bulk_1k")
    names = sorted(f for f in os.listdir(corpus) if f.endswith(".json"))
    print("corpus files:", len(names))
    seeds = [n for n in names if n.startswith(SEEDS)]
    seeds.sort(key=lambda n: 0 if n.startswith("hospital") else 1)
    bundles = []
    for n in names:
        if n.startswith(SEEDS):
            continue
        try:
            with open(os.path.join(corpus, n), encoding="utf-8") as f:
                d = json.load(f)
        except (OSError, ValueError):
            continue
        if isinstance(d, dict) and d.get("resourceType") == "Bundle":
            bundles.append(n)
    keep = bundles[: a.n]
    dst = os.path.join(out, "bundles%d" % a.n)
    shutil.rmtree(dst, ignore_errors=True)
    os.makedirs(dst)
    lines = []
    for n in seeds + keep:
        shutil.copy(os.path.join(corpus, n), dst)
        lines.append("%s  %s\n" % (sha(os.path.join(dst, n)), n))
    mine = "".join(lines)
    with open(os.path.join(dst, "MANIFEST.sha256"), "w") as f:
        f.write(mine)
    print("selected %d seeds + %d bundles -> %s" % (len(seeds), len(keep), dst))
    print("manifest sha256:", hashlib.sha256(mine.encode()).hexdigest())
    if not a.no_verify and a.n == 30:
        exp = open(os.path.join(KIT, "expected", "bundles30.sha256")).read()
        if exp != mine:
            print("MISMATCH with expected/bundles30.sha256 (the public corpus changed or the selection differs)", file=sys.stderr)
            return 1
        print("OK: identical to expected/bundles30.sha256 (sha256 of that file: %s)" % hashlib.sha256(exp.encode()).hexdigest())
    return 0


if __name__ == "__main__":
    sys.exit(main())
