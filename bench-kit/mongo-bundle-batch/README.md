# Mongo bundle-batch ABAB measurement kit (TEMPORARY)

Self-contained kit to reproduce, on another machine, the benchmark that compares `HFS_MONGODB_BUNDLE_BATCH_WRITES=true`
against `origin/main` for the import of 30 fixed Synthea transaction Bundles against MongoDB. Target: a dedicated
Windows machine running **WSL2 (Ubuntu)** with Docker. Everything here is bash + python3 (stdlib only), run **inside WSL**.

This directory is temporary: drop it before opening the PR (`git rm -r bench-kit` or `git revert` the commit that added it).

## What is measured

One run: fresh database -> start `hfs` (MongoDB backend) -> POST 2 seed Bundles (not measured) -> POST 30 patient
transaction Bundles (about 1,600 entries each) with 20 workers over keep-alive connections -> stop `hfs` -> drop the DB -> wait 10 s.
Metrics: total time (first request to last response), resources/s, per-Bundle p50/p95/p99, errors, peak RSS of `hfs` (`VmHWM`),
peak memory and average CPU of the `mongod` container, mongod opcounter deltas (batched writes cut `query` by about 83 %,
`insert` must be identical in both arms). Cap 900 s per run (a run cut by the cap is invalid).

Reference results from the original 8-core Linux box (A = origin/main, B = branch with the flag): total time -35 % to -48 %
(paired), p95 -72 %, hfs peak RSS about +430 MB. Your numbers will differ (hardware); the paired direction should hold.

## 1. Prerequisites (inside WSL2 Ubuntu)

- WSL2 with an Ubuntu distro, `wsl --version` >= 2.x. Keep ALL files (repo, kit, data, Docker volumes) on the Linux filesystem
  (`$HOME/...`), never under `/mnt/c` (slow and distorts timings).
- Memory: give WSL at least 16 GB if the machine has it. Create `%USERPROFILE%\.wslconfig` on Windows, then `wsl --shutdown`:
  ```
  [wsl2]
  memory=20GB      # leave >= 4 GB to Windows; do not leave the default 50 % if it is smaller than ~14 GB
  processors=8     # or all cores; record it in the report
  swap=0           # swap hides memory pressure and distorts timings
  ```
  The MongoDB container is capped at 6 GB and hfs at about 3 GB of RSS: the guard `--min-avail-mb` protects the rest.
- Packages: `sudo apt update && sudo apt install -y build-essential git curl pkg-config libssl-dev clang lld python3`
- Rust toolchain 1.90 or newer (edition 2024): `curl https://sh.rustup.rs -sSf | sh`, then `rustup default stable`.
  Optional, faster linking: `~/.cargo/config.toml` with
  `[target.x86_64-unknown-linux-gnu]` / `linker = "clang"` / `rustflags = ["-C","link-arg=-fuse-ld=lld"]`.
- k6 is NOT needed: this harness drives the import itself (python threads).
- Docker, one of:
  - **Docker Desktop** for Windows with Settings > Resources > WSL integration enabled for your Ubuntu distro. `docker ps` works inside WSL.
    Docker Desktop's VM shares the WSL2 VM memory limit from `.wslconfig`; localhost port forwarding works.
  - **docker engine inside WSL** (no Docker Desktop): follow Docker's "Install on Ubuntu" guide, then
    `sudo usermod -aG docker $USER` and restart the distro (enable systemd in `/etc/wsl.conf`: `[boot]` / `systemd=true`).
  Either way only the `docker` CLI is used.

## 2. Get the code and build the binaries

```bash
git clone https://github.com/HeliosSoftware/hfs.git ~/src/hfs && cd ~/src/hfs
# The branch is LOCAL on the author's box (not pushed). Get it as a git bundle made there:
#   git bundle create mongo-bundle-batch.bundle origin/main..pr/mongo-bundle-batch-final   (copy the file into WSL)
git fetch ~/mongo-bundle-batch.bundle pr/mongo-bundle-batch-final:pr/mongo-bundle-batch-final
git checkout pr/mongo-bundle-batch-final
cd bench-kit/mongo-bundle-batch
export HFS_REPO=$HOME/src/hfs                       # used for --hfs-data-dir (search-parameters-r4.json is read from <repo>/data)

# baseline = origin/main at 678fc5061 ; ours = the branch tip abc74288e (same features for every arm)
bin/build-binaries.sh $HFS_REPO main=678fc5061 ours=abc74288e
# -> bins/hfs-main  bins/hfs-ours  (each built in its own git worktree under work/; first build takes 15-40 minutes)
```

Default features are the original ones (`R4,sqlite,postgres,mongodb,elasticsearch`). A shorter build (`FEATURES=R4,mongodb`) runs the
same Mongo path, but then use it for every arm. If `678fc5061` is not reachable, `git fetch origin main` first. Build time is
not part of the measurement; do not run anything else while timing.

## 3. Start MongoDB (same container as the original runs)

```bash
bin/start-mongo.sh        # container hfs-bench-mongo, 127.0.0.1:27017, mongo:7.0
```

Exact flags (see the script): `docker run -d --memory 6g --memory-swap 6g --ulimit nofile=524288:524288 -v <fresh volume>:/data/db
-p 127.0.0.1:27017:27017 mongo:7.0 --replSet rs0 --bind_ip_all --wiredTigerCacheSizeGB 2 --oplogSize 2048
--setParameter transactionLifetimeLimitSeconds=900`, then `rs.initiate({_id:"rs0",members:[{_id:0,host:"localhost:27017"}]})`
(the member host is `localhost:27017`, so hfs connects with `directConnection=true`; the driver sets the URL
`mongodb://127.0.0.1:<port>/<db>?replicaSet=rs0&directConnection=true`). Original box: container cap 6 GB, WiredTiger cache 2 GB, oplog 2048 MB,
transaction lifetime 900 s (the CI values of `.github/scripts/fhir-bench/start-mongodb.sh`). Environment knobs:
`MONGO_CONTAINER`, `MONGO_PORT`, `MONGO_WT_CACHE_GB`, `MONGO_MEM_LIMIT`. Only the benchmark may use this MongoDB.

## 4. Get the dataset (30 fixed Bundles)

```bash
python3 -I bin/prepare-dataset.py            # downloads ~400 MB, extracts, selects, verifies
```

Source: the public Synthea `bulk_1k` corpus used by the benchmark workflow (`.github/workflows/fhir-benchmark.yml`, tgz server step):
`https://storage.googleapis.com/aidbox-public/synthea/performance/bulk_1k.tar.gz` (1,129 `.json` files). Selection (deterministic):
the 2 seeds (`hospitalInformation*`, `practitionerInformation*`, posted first, not measured), then the first 30 files that
parse as a FHIR Bundle in plain sorted filename order (the original 10 % slice was the first 100; the ABAB used `BUNDLE_N=30`).
The script writes `data/bundles30/` and compares names and per-file SHA-256 with `expected/bundles30.sha256`
(sha256 of that file: `8a395687586bf813b594544a9c7b83a148d84c3f81c991d756cc7fd2e712c9c6`; sha256 of just the 32 names:
`fb4ae48160bcd8632f0d8800c4d55f402c4ecb60310a7fa8de87c4f5eaf9c64f`). It prints `OK` or `MISMATCH`; do not measure on a mismatch.
The original run used a tarball with sha256 `7844b5f8c0bf9a05cf8ea9557b353ce96c841a3c8ac397123a97dc54c51eaf5f`.

## 5. Record the hardware (once per session, and again if anything changes)

```bash
bin/hwinfo.sh hfs-bench-mongo > results/hardware.md
```
It records CPU model/cores, RAM seen by the VM, disk (add the physical disk type by hand, a VM always reports a virtual disk),
kernel, Docker flavour/version, the Mongo container's exact flags, version, rs host, transaction limit, WiredTiger cache, oplog,
toolchain, load. On Windows interop it also tries `ver`, `wsl --version` and `.wslconfig`; if the output says interop is disabled,
paste by hand: Windows build (`winver`), `wsl --version`, the contents of `%USERPROFILE%\.wslconfig`, power plan (use High performance,
plugged in), and whether antivirus real-time scanning excludes the WSL VHDX.

## 6. Run ABAB x3

Quiet machine: close browsers/IDEs, no Windows Update, plugged in. One run at a time, never two MongoDB users.

```bash
python3 -I bin/abab.py --out results/abab1 --rounds 3 --min-avail-mb 2048 \
  --arm main=bins/hfs-main \
  --arm ours=bins/hfs-ours --env ours:HFS_MONGODB_BUNDLE_BATCH_WRITES=true
python3 -I bin/summary.py results/abab1 --md results/abab1/summary.md
```
`--hfs-data-dir` defaults to `$HFS_REPO/data`. The plan is A B A B A B (use `--rotate` to rotate the order each round for
three or more arms). Every run: fresh database `kit_<arm>_r<round>_<epoch>`, dropped afterwards, then a 10 s wait. The driver refuses
to start if a leftover `kit_*` database exists. Each run writes `results/abab1/<arm>-r<N>/{run.json,hfs.log,hfs.env}`.
Expect about 20-45 s per run on the original box (slower machines: minutes); the cap is 900 s.
Use `--dry-run` to see the plan, `--resume` to skip runs that already have `run.json`. Smoke test first:
`python3 -I bin/prepare-dataset.py --n 2 --no-verify` then `--data data/bundles2 --rounds 1 --cap-s 150`.

Validity rules: a run with errors, a cap cut, a crashed hfs, or a memory-guard kill is marked INVALID and the summary drops
that whole round for that arm pair; re-run it with `--resume`. Record `loadavg_before` (printed in the table) and any foreign load.

## 7. Memory sampling

Every 3 s (`--sample-s`): `hfs` RSS from `/proc/<pid>/status` (peak = `VmHWM`), `mongod` container memory and CPU from
`docker stats --no-stream` (works with Docker Desktop and docker-in-WSL), plus MemAvailable of the WSL VM (minimum recorded as
`min_mem_available_mb`). `--min-avail-mb N` is the optional guard: it kills the hfs it started if MemAvailable falls below N.

## 8. Statistics

`bin/summary.py` pairs round k of each arm with round k of the baseline arm (the first `--arm`), computes the paired percent
difference per metric, their median and mean, and a bootstrap 95 % CI of the median (10,000 resamples of the pairs, percentile method,
fixed seed). Verdict text per metric: direction, whether every pair agrees, whether all runs of the arm beat all baseline runs, whether
the CI excludes 0 (only claimed with >= 3 pairs), and the baseline-baseline spread. With 3 pairs the CI can only span the observed differences:
treat it as a sanity check and add rounds (`--rounds 5`) if it crosses 0. Accept an improvement only if every
pair agrees AND the median gain exceeds the baseline spread. It also accepts `--csv` (label,round,total_s,...) to analyse numbers from elsewhere.

## 9. Measuring an upstream PR branch (e.g. #2144)

An open upstream PR (#2144, "perf(mongodb): stage transaction Bundle creates and send them in chunks", always on) overlaps with this
branch. Add it as one more arm; the driver and summary accept any number of arms:

```bash
git -C $HFS_REPO fetch origin pull/2144/head:pr-2144
bin/build-binaries.sh $HFS_REPO pr2144=pr-2144
python3 -I bin/abab.py --out results/abab2 --rounds 3 --min-avail-mb 2048 --rotate \
  --arm main=bins/hfs-main \
  --arm ours=bins/hfs-ours --env ours:HFS_MONGODB_BUNDLE_BATCH_WRITES=true \
  --arm pr2144=bins/hfs-pr2144
python3 -I bin/summary.py results/abab2 --md results/abab2/summary.md
```
The summary prints `ours vs main` and `pr2144 vs main`; to compare ours directly against the PR, use `--baseline pr2144` (or run
the summary twice). Check the PR's description for any env var it needs and add it with `--env pr2144:KEY=VAL`. If the PR
branch is not based on 678fc5061, say so in the report (a different base is a confounder): build `main` from the PR's merge base too.
Compare opcounters (`run.json: opcounters_delta`): identical `insert` counts mean the same documents were written.

## 10. What to send back

`results/<session>/` (all `run.json`, `summary.md`, `session.json`, hfs logs), `results/hardware.md`, the output of
`bin/prepare-dataset.py` (OK line), `bins/builds.txt`, and the exact commands used.

## Layout

`bin/hwinfo.sh` hardware collector - `bin/start-mongo.sh` MongoDB replica set - `bin/build-binaries.sh` per-commit builds -
`bin/prepare-dataset.py` dataset selection/verification - `bin/abab.py` driver - `bin/summary.py` statistics -
`expected/bundles30.sha256` the manifest of the 30 Bundles + seeds. `data/ results/ bins/ work/` are git-ignored.
