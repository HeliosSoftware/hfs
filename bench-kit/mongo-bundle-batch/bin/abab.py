#!/usr/bin/env python3
"""Binary-agnostic interleaved (ABAB...) driver for the MongoDB transaction-Bundle import benchmark. Stdlib only, Linux/WSL.

One run = fresh database -> start hfs (one binary + per-arm env) -> POST the 2 seed Bundles -> POST the N patient Bundles
(default 30) as transaction Bundles with 20 worker threads (one keep-alive connection each) -> stop hfs -> drop the database
-> wait 10 s. Rounds interleave the arms: round 1 = arm1, arm2, arm3 ...; round 2 = the same order again, etc.
The FIRST --arm is the baseline the summary compares everything against. One run at a time, never two MongoDB users.

  python3 -I bin/abab.py --out results/session1 --rounds 3 \\
      --arm main=bins/hfs-main \\
      --arm ours=bins/hfs-ours --env ours:HFS_MONGODB_BUNDLE_BATCH_WRITES=true \\
      --arm pr2144=bins/hfs-pr2144            # optional third arm (see README)

Required: --hfs-data-dir <repo>/data (or env HFS_REPO=<repo>).
Options: --data DIR (default data/bundles30)  --bundles N  --vus 20  --cap-s 900  --mongo-container  --mongo-port
  --db-prefix kit_  --settle-s 10  --min-avail-mb 0 (memory guard: kill hfs and mark the run invalid below this MemAvailable)
  --sample-s 3  --dry-run  --resume (skip runs whose run.json exists)  --rotate (rotate the arm order each round)
Run `python3 -I bin/summary.py <out>` afterwards.
"""
import argparse, glob, http.client, json, os, signal, socket, subprocess, sys, threading, time, urllib.request

SEEDS = ("hospitalInformation", "practitionerInformation")
now = time.time


def pct(vals, p):
    if not vals:
        return None
    v = sorted(vals)
    k = (len(v) - 1) * p / 100.0
    lo = int(k)
    hi = min(lo + 1, len(v) - 1)
    return round(v[lo] + (v[hi] - v[lo]) * (k - lo), 2)


def sh(cmd, timeout=120):
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout).stdout
    except Exception as e:
        return ""


class Mongo:
    def __init__(self, container):
        self.c = container

    def js(self, code, db=None, timeout=120):
        cmd = ["docker", "exec", self.c, "mongosh", "--quiet"] + ([db] if db else []) + ["--eval", code]
        return sh(cmd, timeout).strip()

    def dbs(self, prefix):
        return [d for d in self.js('db.adminCommand({listDatabases:1}).databases.map(d=>d.name).join(" ")').split() if d.startswith(prefix)]

    def drop(self, db):
        self.js("db.dropDatabase()", db)

    def opcounters(self):
        try:
            return json.loads(self.js("var o=db.serverStatus().opcounters; var r={}; Object.keys(o).forEach(k=>r[k]=Number(o[k])); JSON.stringify(r)"))
        except Exception:
            return {}

    def version(self):
        return self.js("db.version()")

    def stats(self):
        """(mem MB, cpu cores) of the container from `docker stats` (works for Docker Desktop and docker-in-WSL)."""
        o = sh(["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}|{{.CPUPerc}}", self.c], 20).strip()
        try:
            m, c = o.split("|")
            used = m.split("/")[0].strip()
            mult = {"KiB": 1 / 1024, "MiB": 1, "GiB": 1024, "TiB": 1048576, "kB": 1 / 1024, "MB": 1, "GB": 1024, "B": 1 / 1048576}
            for u in sorted(mult, key=len, reverse=True):
                if used.endswith(u):
                    return float(used[: -len(u)]) * mult[u], float(c.strip().rstrip("%")) / 100.0
        except Exception:
            pass
        return None


def mem_available_mb():
    for l in open("/proc/meminfo"):
        if l.startswith("MemAvailable"):
            return int(l.split()[1]) // 1024
    return 10**9


def proc_cpu_s(pid):
    f = open("/proc/%d/stat" % pid).read().rsplit(")", 1)[1].split()
    return (int(f[11]) + int(f[12])) / os.sysconf("SC_CLK_TCK")


def proc_rss_hwm(pid):
    rss = hwm = 0
    for l in open("/proc/%d/status" % pid):
        if l.startswith("VmRSS"):
            rss = int(l.split()[1])
        elif l.startswith("VmHWM"):
            hwm = int(l.split()[1])
    return rss / 1024.0, hwm / 1024.0


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class Sampler(threading.Thread):
    def __init__(self, pid, mongo, interval, min_avail):
        super().__init__(daemon=True)
        self.pid, self.mongo, self.iv, self.min_avail = pid, mongo, interval, min_avail
        self.stop_ev = threading.Event()
        self.hfs_rss, self.hfs_hwm, self.mongo_mem, self.mongo_cpu, self.min_seen = 0.0, 0.0, 0.0, [], 10**9
        self.guard_fired = None
        self.cpu0 = self.t0 = None

    def run(self):
        try:
            self.cpu0, self.t0 = proc_cpu_s(self.pid), now()
        except Exception:
            pass
        while not self.stop_ev.is_set():
            try:
                rss, hwm = proc_rss_hwm(self.pid)
                self.hfs_rss, self.hfs_hwm = max(self.hfs_rss, rss), max(self.hfs_hwm, hwm)
            except Exception:
                pass
            av = mem_available_mb()
            self.min_seen = min(self.min_seen, av)
            if self.min_avail and av < self.min_avail:
                self.guard_fired = "MemAvailable %d MB < %d MB" % (av, self.min_avail)
                try:
                    os.kill(self.pid, signal.SIGKILL)
                except Exception:
                    pass
                return
            st = self.mongo.stats()
            if st:
                self.mongo_mem = max(self.mongo_mem, st[0])
                self.mongo_cpu.append(st[1])
            self.stop_ev.wait(self.iv)

    def finish(self):
        self.stop_ev.set()
        self.join(30)
        try:
            cpu = (proc_cpu_s(self.pid) - self.cpu0) / max(now() - self.t0, 1e-9)
        except Exception:
            cpu = None
        return cpu


def read_seeds_bundles(d, n):
    files = sorted(glob.glob(os.path.join(d, "*.json")))
    seeds = [f for f in files if os.path.basename(f).startswith(SEEDS)]
    seeds.sort(key=lambda f: 0 if "hospital" in os.path.basename(f) else 1)
    b = [f for f in files if f not in seeds]
    return seeds, (b[:n] if n else b)


def import_phase(port, seeds, files, vus, deadline):
    hdr = {"Content-Type": "application/json", "Accept": "application/json"}
    for f in seeds:
        c = http.client.HTTPConnection("127.0.0.1", port, timeout=max(5, deadline - now()))
        c.request("POST", "/", open(f, "rb").read(), hdr)
        r = c.getresponse()
        r.read()
        c.close()
        if r.status != 200:
            return {"status": "error", "reason": "seed %s: HTTP %d" % (os.path.basename(f), r.status)}
    lock, state, recs = threading.Lock(), {"i": 0, "stop": False}, []

    def worker():
        conn = http.client.HTTPConnection("127.0.0.1", port, timeout=900)
        while not state["stop"]:
            with lock:
                if state["i"] >= len(files):
                    return
                f = files[state["i"]]
                state["i"] += 1
            body = open(f, "rb").read()
            try:
                n = len(json.loads(body).get("entry", []))
            except Exception:
                n = 0
            t = now()
            try:
                conn.request("POST", "/", body, hdr)
                r = conn.getresponse()
                rb = r.read()
                st = r.status
                if st != 200 and "err" not in state:
                    state["err"] = "HTTP %d: %s" % (st, rb[:300].decode("utf-8", "replace"))
            except Exception as ex:
                st = 0
                state.setdefault("err", "connection error: %r" % ex)
                conn.close()
                conn = http.client.HTTPConnection("127.0.0.1", port, timeout=900)
            recs.append((now(), (now() - t) * 1000, st, n))

    t0 = now()
    ths = [threading.Thread(target=worker, daemon=True) for _ in range(vus)]
    for t in ths:
        t.start()
    while now() < deadline and any(t.is_alive() for t in ths):
        time.sleep(0.2)
    cut = now() >= deadline and any(t.is_alive() for t in ths)
    state["stop"] = True
    t_cut = min(now(), deadline)
    end = now() + 10
    for t in ths:
        t.join(max(0, end - now()))
    done = [r for r in recs if r[0] <= t_cut + 0.001] if cut else list(recs)
    ok = [r for r in done if r[2] == 200]
    window = (t_cut - t0) if cut else ((max(r[0] for r in done) - t0) if done else 0)
    res_ok = sum(r[3] for r in ok)
    lat = [r[1] for r in ok]
    return {"status": "partial" if cut else "complete", "total_s": round(window, 2), "rate_res_s": round(res_ok / window, 1) if window > 0 else 0,
            "p50_ms": pct(lat, 50), "p95_ms": pct(lat, 95), "p99_ms": pct(lat, 99), "errors": len(done) - len(ok),
            "first_error": state.get("err"), "bundles_ok": len(ok), "bundles_planned": len(files), "resources_ok": res_ok}


def one_run(a, mongo, label, binary, env, rnd, idx, seeds, files):
    name = "%s-r%d" % (label, rnd)
    rdir = os.path.join(a.out, name)
    if a.resume and os.path.exists(os.path.join(rdir, "run.json")):
        print("[%s] exists, skipped (--resume)" % name)
        return json.load(open(os.path.join(rdir, "run.json")))
    os.makedirs(os.path.join(rdir, "cwd"), exist_ok=True)
    left = mongo.dbs(a.db_prefix)
    if left:
        sys.exit("refusing to start: leftover databases with prefix %s: %s (someone else using this MongoDB? drop them first)" % (a.db_prefix, left))
    db = "%s%s_%d" % (a.db_prefix, name.replace("-", "_").replace(".", "_"), int(now()))
    port = a.hfs_port or free_port()
    e = {k: v for k, v in os.environ.items() if not k.startswith(("HFS_", "FHIRPATH_"))}
    e.update({"HFS_STORAGE_BACKEND": "mongodb", "HFS_MONGODB_URL": "mongodb://127.0.0.1:%d/%s?replicaSet=rs0&directConnection=true" % (a.mongo_port, db),
              "HFS_MONGODB_DATABASE": db, "HFS_SERVER_HOST": "127.0.0.1", "HFS_SERVER_PORT": str(port), "HFS_BASE_URL": "http://127.0.0.1:%d" % port,
              "HFS_LOG_LEVEL": "info", "HFS_MAX_BODY_SIZE": "209715200", "HFS_REQUEST_TIMEOUT": "900", "HFS_MONGODB_MAX_CONNECTIONS": "32"})
    e["HFS_DATA_DIR"] = a.hfs_data_dir  # search-parameters-r4.json etc.; without it conditional references never resolve
    e.update(env)
    with open(os.path.join(rdir, "hfs.env"), "w") as f:
        for k in sorted(e):
            if k.startswith("HFS_"):
                f.write("%s=%s\n" % (k, e[k]))
    rec = {"name": name, "label": label, "round": rnd, "order": idx, "binary": os.path.abspath(binary), "env": env, "db": db, "port": port,
           "loadavg_before": open("/proc/loadavg").read().strip(), "t0": int(now()), "valid": False}
    print("[%s] db=%s port=%d binary=%s env=%s" % (name, db, port, binary, env or "-"), flush=True)
    log = open(os.path.join(rdir, "hfs.log"), "w")
    p = subprocess.Popen([os.path.abspath(binary)], cwd=os.path.join(rdir, "cwd"), env=e, stdout=log, stderr=subprocess.STDOUT)
    sm = None
    try:
        for _ in range(a.ready_timeout):
            if p.poll() is not None:
                raise RuntimeError("hfs exited during startup (see %s/hfs.log)" % rdir)
            try:
                urllib.request.urlopen("http://127.0.0.1:%d/metadata" % port, timeout=2).read(1)
                break
            except Exception:
                time.sleep(1)
        else:
            raise RuntimeError("hfs not ready in %d s" % a.ready_timeout)
        sm = Sampler(p.pid, mongo, a.sample_s, a.min_avail_mb)
        sm.start()
        ops0 = mongo.opcounters()
        res = import_phase(port, seeds, files, a.vus, now() + a.cap_s)
        ops1 = mongo.opcounters()
        rec.update(res)
        rec["opcounters_delta"] = {k: ops1[k] - ops0.get(k, 0) for k in ops1 if isinstance(ops1[k], (int, float))}
        rec["valid"] = res.get("status") == "complete" and res.get("errors", 1) == 0
        if not rec["valid"]:
            rec["invalid_reason"] = "status=%s errors=%s %s %s" % (res.get("status"), res.get("errors"), res.get("reason", ""), res.get("first_error") or "")
    except Exception as ex:
        rec["invalid_reason"] = "exception: %s" % ex
    finally:
        if sm:
            rec["hfs_cpu_avg_cores"] = sm.finish()
            rec["hfs_rss_peak_mb"] = round(sm.hfs_hwm or sm.hfs_rss, 1)
            rec["mongo_mem_peak_mb"] = round(sm.mongo_mem, 1) if sm.mongo_mem else None
            rec["mongo_cpu_avg_cores"] = round(sum(sm.mongo_cpu) / len(sm.mongo_cpu), 2) if sm.mongo_cpu else None
            rec["min_mem_available_mb"] = sm.min_seen
            if sm.guard_fired:
                rec["valid"] = False
                rec["invalid_reason"] = "memguard: " + sm.guard_fired
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(30)
            except Exception:
                p.kill()
        log.close()
        mongo.drop(db)
        rec["db_dropped"] = db not in mongo.dbs(a.db_prefix)
    with open(os.path.join(rdir, "run.json"), "w") as f:
        json.dump(rec, f, indent=1)
    print("[%s] %s total_s=%s rate=%s p50=%s p95=%s errors=%s hfs_rss=%s mongo_mem=%s %s" % (
        name, "VALID" if rec["valid"] else "INVALID", rec.get("total_s"), rec.get("rate_res_s"), rec.get("p50_ms"), rec.get("p95_ms"),
        rec.get("errors"), rec.get("hfs_rss_peak_mb"), rec.get("mongo_mem_peak_mb"), rec.get("invalid_reason", "")), flush=True)
    print("[%s] waiting %d s" % (name, a.settle_s), flush=True)
    time.sleep(a.settle_s)
    return rec


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    kit = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    ap.add_argument("--arm", action="append", default=[], metavar="LABEL=BINARY", help="repeatable; the first one is the baseline")
    ap.add_argument("--env", action="append", default=[], metavar="LABEL:KEY=VAL", help="per-arm environment variable (repeatable)")
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--out", default=os.path.join(kit, "results", time.strftime("session-%Y%m%d-%H%M%S")))
    ap.add_argument("--data", default=os.path.join(kit, "data", "bundles30"))
    ap.add_argument("--bundles", type=int, default=0, help="use only the first N patient Bundles (0 = all in --data; the real ABAB uses 30)")
    ap.add_argument("--vus", type=int, default=20)
    ap.add_argument("--cap-s", type=int, default=900)
    ap.add_argument("--mongo-container", default=os.environ.get("MONGO_CONTAINER", "hfs-bench-mongo"))
    ap.add_argument("--mongo-port", type=int, default=int(os.environ.get("MONGO_PORT", "27017")))
    ap.add_argument("--hfs-data-dir", default=os.environ.get("HFS_DATA_DIR") or (os.path.join(os.environ["HFS_REPO"], "data") if os.environ.get("HFS_REPO") else ""),
                    help="the repository's data/ directory (search-parameters-r4.json...); default $HFS_DATA_DIR or $HFS_REPO/data")
    ap.add_argument("--hfs-port", type=int, default=0, help="0 = pick a free port")
    ap.add_argument("--db-prefix", default="kit_")
    ap.add_argument("--settle-s", type=int, default=10)
    ap.add_argument("--ready-timeout", type=int, default=120)
    ap.add_argument("--sample-s", type=float, default=3)
    ap.add_argument("--min-avail-mb", type=int, default=0, help="memory guard threshold in MB (0 = off); recommended: 2048 on a 16 GB machine")
    ap.add_argument("--rotate", action="store_true", help="rotate the arm order every round (A B C / B C A / C A B) to spread order effects")
    ap.add_argument("--resume", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    if len(a.arm) < 2:
        ap.error("need at least two --arm LABEL=BINARY (the first is the baseline)")
    arms = []
    for s in a.arm:
        l, _, b = s.partition("=")
        if not l or not b or not all(c.isalnum() or c in "._-" for c in l):
            ap.error("bad --arm %r" % s)
        arms.append((l, b))
    if len({l for l, _ in arms}) != len(arms):
        ap.error("duplicate arm labels")
    envs = {l: {} for l, _ in arms}
    for s in a.env:
        l, _, kv = s.partition(":")
        k, _, v = kv.partition("=")
        if l not in envs or not k:
            ap.error("bad --env %r (label must be one of the --arm labels)" % s)
        envs[l][k] = v
    if not os.path.exists(os.path.join(a.hfs_data_dir or "/nonexistent", "search-parameters-r4.json")):
        ap.error("--hfs-data-dir (or env HFS_REPO=<repo checkout>) must point at the repo's data/ dir containing search-parameters-r4.json")
    a.hfs_data_dir = os.path.abspath(a.hfs_data_dir)
    seeds, files = read_seeds_bundles(a.data, a.bundles)
    plan = []
    for r in range(1, a.rounds + 1):
        order = arms[(r - 1) % len(arms):] + arms[: (r - 1) % len(arms)] if a.rotate else arms
        plan += [(l, b, r) for l, b in order]
    print("baseline: %s | data: %s (%d seeds + %d bundles) | vus=%d cap=%ds | out=%s" % (arms[0][0], a.data, len(seeds), len(files), a.vus, a.cap_s, a.out))
    for i, (l, b, r) in enumerate(plan):
        print("  %2d. round %d arm %-10s %s env=%s" % (i + 1, r, l, b, envs[l] or "-"))
    if a.dry_run:
        return 0
    if not files or not seeds:
        sys.exit("no seeds/bundles in %s: run bin/prepare-dataset.py first" % a.data)
    for l, b in arms:
        if not os.access(b, os.X_OK):
            sys.exit("binary not executable: %s" % b)
    mongo = Mongo(a.mongo_container)
    if not mongo.version():
        sys.exit("cannot talk to MongoDB in container %s (docker exec mongosh failed): run bin/start-mongo.sh" % a.mongo_container)
    os.makedirs(a.out, exist_ok=True)
    json.dump({"arms": [{"label": l, "binary": os.path.abspath(b), "env": envs[l]} for l, b in arms], "baseline": arms[0][0], "rounds": a.rounds,
               "bundles": len(files), "vus": a.vus, "cap_s": a.cap_s, "mongo_version": mongo.version(), "started": time.strftime("%FT%TZ", time.gmtime())},
              open(os.path.join(a.out, "session.json"), "w"), indent=1)
    for i, (l, b, r) in enumerate(plan):
        rec = one_run(a, mongo, l, b, envs[l], r, i, seeds, files)
        if rec.get("invalid_reason") and not rec.get("valid"):
            print("run invalid; continuing (the summary drops the whole round). Re-run just it with --resume after fixing the cause.")
    print("done. Next: python3 -I bin/summary.py %s" % a.out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
