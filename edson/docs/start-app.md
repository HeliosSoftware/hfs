# Starting the Helios FHIR Server (HFS) with the Web UI

Session notes for bringing up `hfs` on a fresh Windows box, from an empty
toolchain to a live UI at `http://127.0.0.1:8080/ui`.

- **Project root:** `C:\Users\tercere\src\helios\hfs`
- **Repo:** [HeliosSoftware/hfs](https://github.com/HeliosSoftware/hfs) — Rust workspace, 20 crates, edition 2024, MSRV 1.90
- **Goal reached:** dashboard live in the browser after a first-time build
- **Total elapsed:** ~30 minutes wall-clock, dominated by the fresh `cargo build`

---

## 1. Findings before starting

### Project shape
- Rust workspace with 20 crates under `crates/`. Default-members skip `pysof` so
  a bare `cargo build` doesn't need Python.
- The main FHIR server binary is `hfs` (`crates/hfs`), which mounts the web UI
  from `crates/ui` under `/ui` when built with the `ui` feature — which is on
  by default. `helios-hfs` + `--features headless` disables the UI (headless
  wins over ui).
- Other binaries in the workspace: `hts` (terminology server, port 8090),
  `fhirpath-cli`, `fhirpath-server`, `sof-cli`, `sof-server`,
  `config-advisor`, `validator-cli`.
- Config is entirely env-var driven (`HFS_*`, documented in
  `crates/hfs/README.md` and `.claude/skills/run-hfs-server/SKILL.md`).
- Default features build **FHIR R4 only**. Enabling R6 triggers
  `helios-fhir-gen` to download R6 specs from `build.fhir.org` — worth
  skipping on the first build.

### Environment probe
| Tool | Result |
|---|---|
| Rust (`rustc`, `cargo`, `rustup`) | **Missing** — had to install |
| Git 2.53.0 | Present |
| Docker 28.0.1 | Present (unused in this run) |
| JDK 25 auto-loaded by PowerShell profile | Irrelevant to this project |
| VS 2022 Build Tools (`vswhere.exe`) | **Missing** |
| winget | **Missing** |
| Disk free on C: | 408 GB (plenty) |

### Corporate proxy constraint
- The `HTTP_PROXY` / `HTTPS_PROXY` / `http_proxy` / `https_proxy` env vars
  point at `dfwproxy.ent.covance.com:80`, which is unreachable off VPN.
- Cursor rule `~/.cursor/rules/corporate-proxy-bypass.mdc` documents the same
  problem for `pnpm`: **clear the four proxy env vars in the session and reach
  the internet directly**.
- That same trick works verbatim for **every step of the Rust install chain**:
  - `rustup-init.exe` from `win.rustup.rs`
  - `rustup` fetching toolchains from `static.rust-lang.org`
  - `cargo` fetching crates from `index.crates.io` / `static.crates.io`
  - Direct GitHub release downloads for mingw-w64
- Because the bypass goes direct to the internet (not through a MITM proxy),
  no corporate root CA import is needed for TLS.
- **Do not clear proxy vars for `git`** — its config may use them intentionally.

### Session gotcha (important)
- Under this shell integration, **each command runs in a fresh PowerShell** that
  re-loads `$PROFILE`, which re-injects the proxy env vars. A one-time
  `Remove-Item Env:HTTP_PROXY,...` at the top of the session does **not**
  persist to subsequent commands.
- **Prepend the proxy clear to every command** that touches the network or
  runs cargo/rustup.

---

## 2. Deviations from the original plan

The plan targeted the **MSVC** toolchain (`stable-x86_64-pc-windows-msvc`). Two
deviations were needed:

### 2a. Switched to the GNU toolchain
- MSVC needs `link.exe` from Visual Studio Build Tools with the "Desktop
  development with C++" workload — not installed, and installing it means
  4–8 GB of download and typically admin rights.
- `rustup` also installs `stable-x86_64-pc-windows-gnu`, which ships its own
  linker (`rust-lld.exe`) — no admin, no MSVC needed.
- Command used: `rustup default stable-x86_64-pc-windows-gnu`.

### 2b. Installed portable mingw-w64 (winlibs GCC 16.2.0)
- First `cargo build` with the GNU toolchain still failed:
  ```
  error: error calling dlltool 'dlltool.exe': program not found
  error: could not compile `getrandom` (lib) due to 1 previous error
  ```
- `rust-lld.exe` alone isn't enough — the `getrandom` crate's build script
  calls `dlltool.exe` to generate a Windows import library, and other crates
  using `cc` (SQLite, ring, etc.) need `gcc.exe` / `ar.exe` / `windres.exe`.
- Installed the portable winlibs zip (POSIX threads, SEH exceptions, UCRT):
  - **Package:** `winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r1.zip`
  - **Source:** GitHub `brechtsanders/winlibs_mingw` release
    `16.2.0posix-14.0.0-ucrt-r1`
  - **Size:** 261 MB
  - **Install path:** `C:\Users\tercere\mingw64-toolchain\mingw64\`
    with `bin/` on PATH — contains `gcc`, `g++`, `ld`, `ar`, `dlltool`,
    `windres`, `ranlib`
- No admin rights required — just an extracted zip.

---

## 3. Reproducible install steps

All commands run in **PowerShell 5+/7 on Windows**. Every network-touching
command starts with the proxy clear because the shell profile re-injects the
proxy vars on each fresh invocation.

### Step 1 — Proxy bypass sanity check
```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
[System.Net.Dns]::GetHostAddresses('static.rust-lang.org') | Select-Object -First 1
```
Expected: an IP in the `151.101.x.x` range (Fastly CDN) or similar. Anything
that returns `dfwproxy.ent.covance.com` means the clear didn't take.

### Step 2 — Install Rust
```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
$ProgressPreference = 'SilentlyContinue'
Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile "$env:TEMP\rustup-init.exe" -UseBasicParsing
& "$env:TEMP\rustup-init.exe" -y --default-host x86_64-pc-windows-msvc --default-toolchain stable --profile default
```
Installs `rustc 1.97.1` under `%USERPROFILE%\.cargo\`. rustup-init prints a
warning about missing MSVC prerequisites — that's fine; we'll switch to GNU
in step 3.

### Step 3 — Switch to the GNU toolchain
```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
rustup toolchain install stable-x86_64-pc-windows-gnu --profile default
rustup default stable-x86_64-pc-windows-gnu
rustc --version   # expect 1.97.1
```

### Step 4 — Install portable mingw-w64 (winlibs)
```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
$url = 'https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-14.0.0-ucrt-r1/winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r1.zip'
$zip = "$env:TEMP\mingw64.zip"
$dest = "$env:USERPROFILE\mingw64-toolchain"
$ProgressPreference = 'SilentlyContinue'
Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
Expand-Archive -Path $zip -DestinationPath $dest -Force
# Verify: expect $dest\mingw64\bin\dlltool.exe to exist
Test-Path "$dest\mingw64\bin\dlltool.exe"
```
Extract takes several minutes — the archive contains tens of thousands of
files. Once extracted the `bin/` folder holds `gcc`, `g++`, `ld`, `ar`,
`dlltool`, `windres`, `ranlib`, etc.

### Step 5 — Build helios-hfs
```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:USERPROFILE\mingw64-toolchain\mingw64\bin;$env:Path"
Set-Location C:\Users\tercere\src\helios\hfs
cargo build -p helios-hfs
```
- Default features: **R4 + `ui`** — no need to pass `--features` for the UI.
- Debug profile.
- **First build took ~13m 43s** on this box. Produces
  `target\debug\hfs.exe` (~1.1 GB debug binary — normal).
- If cargo stalls or errors on `dfwproxy.ent.covance.com`, the proxy vars got
  re-injected. Re-run the `Remove-Item` line and retry — cargo's
  `%USERPROFILE%\.cargo\registry` cache lets partial downloads resume.

### Step 6 — Run the server
```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:USERPROFILE\mingw64-toolchain\mingw64\bin;$env:Path"
Set-Location C:\Users\tercere\src\helios\hfs
$env:HFS_LOG_LEVEL = "debug"
cargo run -p helios-hfs
# or, equivalent, skip cargo overhead:
.\target\debug\hfs.exe
```
On first launch the server bootstraps the SearchParameter registry into the
SQLite store — this took ~3 minutes with `HFS_LOG_LEVEL=debug`. Wait for:
```
INFO hfs: Server listening address=127.0.0.1:8080
```

### Step 7 — Verify
```powershell
Invoke-WebRequest http://127.0.0.1:8080/health -UseBasicParsing | Select-Object StatusCode, Content
Invoke-WebRequest http://127.0.0.1:8080/metadata -Headers @{Accept='application/fhir+json'} -UseBasicParsing | Select-Object StatusCode, @{n='CT';e={$_.Headers['Content-Type']}}, RawContentLength
Invoke-WebRequest http://127.0.0.1:8080/ui -UseBasicParsing | Select-Object StatusCode, RawContentLength
Start-Process 'http://127.0.0.1:8080/ui'
```

---

## 4. Results captured in this session

- **Rust:** `rustc 1.97.1 (8bab26f4f 2026-07-14)` / `cargo 1.97.1`
- **Active toolchain:** `stable-x86_64-pc-windows-gnu` (default)
- **Also installed:** `stable-x86_64-pc-windows-msvc` (unused; leave for later
  if you want to migrate to MSVC after installing VS Build Tools)
- **GCC:** `MinGW-W64 x86_64-ucrt-posix-seh 16.2.0` at
  `C:\Users\tercere\mingw64-toolchain\mingw64\bin\gcc.exe`
- **Build time:** 13 minutes 43 seconds (fresh, debug, R4 + ui)
- **Binary:** `C:\Users\tercere\src\helios\hfs\target\debug\hfs.exe` (~1.1 GB)
- **Server started:** `2026-08-17T22:33:34Z`
- **Listening:** `127.0.0.1:8080`
- **Storage backend:** SQLite (default), file `.\fhir.db` in the working
  directory
- **FHIR version:** R4 (default)

### Smoke-test responses
| Endpoint | Status | Notable |
|---|---|---|
| `GET /health` | 200 | `{"status":"ok","service":"hfs","version":"0.2.1", ...}` |
| `GET /metadata` | 200 | `application/fhir+json; fhirVersion=4.0`, ~2.5 MB CapabilityStatement |
| `GET /ui` | 200 | `text/html; charset=utf-8`, ~42 KB, page title **"Helios FHIR Server"** |

---

## 5. UI routes worth trying

All under `http://127.0.0.1:8080`:

| Path | Page |
|---|---|
| `/ui` | Dashboard (stat cards, resources-over-time chart) |
| `/ui/resources` | Resources workspace — type rail, search, edit modal |
| `/ui/editor` | Schema-driven resource editor |
| `/ui/queries` | Saved queries + visual search builder |
| `/ui/search-parameters` | SearchParameter viewer/CRUD |
| `/ui/compartments` | Compartment viewer + membership tester |
| `/ui/history` | Version rail + server-side diff |
| `/ui/batch` | Batch/Transaction workspace |
| `/ui/tenants` | Tenant maintenance |
| `/ui/status` | Fragment-vs-full-page demo |
| `/metadata` | Raw FHIR CapabilityStatement (not under `/ui`) |

The UI is server-rendered Askama + htmx (no React, no bundler, no npm at
runtime). Editing anything under `crates/ui/assets/` or `crates/ui/templates/`
needs a rebuild — assets are embedded via `rust-embed` at compile time.

---

## 6. Environment variables used

Only two set for this session, both optional:

| Var | Value | Effect |
|---|---|---|
| `HFS_LOG_LEVEL` | `debug` | Verbose tracing (default is `info`) |
| `PATH` | prepended with cargo + mingw bin | Needed to run rustc/cargo and to invoke gcc/dlltool during builds |

Everything else defaulted:
- `HFS_STORAGE_BACKEND=sqlite`
- `HFS_DATABASE_URL=fhir.db` (created in the working directory)
- `HFS_SERVER_HOST=127.0.0.1`
- `HFS_SERVER_PORT=8080`
- `HFS_BASE_URL=http://localhost:8080`
- `HFS_DEFAULT_FHIR_VERSION=R4`
- `HFS_ENABLE_CORS=true`

Full env-var reference lives in the project's root
[`README.md`](../../README.md) and in
[`.claude/skills/run-hfs-server/SKILL.md`](../../.claude/skills/run-hfs-server/SKILL.md).

---

## 7. Restart recipe (for later sessions)

Fresh PowerShell terminal:

```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:USERPROFILE\mingw64-toolchain\mingw64\bin;$env:Path"
Set-Location C:\Users\tercere\src\helios\hfs
$env:HFS_LOG_LEVEL = "debug"
cargo run -p helios-hfs
```

Or, if you don't need to rebuild:

```powershell
Remove-Item Env:HTTP_PROXY,Env:HTTPS_PROXY,Env:http_proxy,Env:https_proxy -ErrorAction SilentlyContinue
Set-Location C:\Users\tercere\src\helios\hfs
$env:HFS_LOG_LEVEL = "debug"
.\target\debug\hfs.exe
```
The direct binary launch skips cargo and skips the mingw PATH prepend — the
`.exe` has already been linked and needs nothing but Windows itself at runtime.
The proxy clear is still worthwhile in case anything the server calls out to
uses those env vars.

---

## 8. Optional follow-ups (not done this session)

- **Faster relinks with LLD:** add the `lld-link.exe` block from the
  project `README.md` to `%USERPROFILE%\.cargo\config.toml` after installing
  LLVM. Optional; only helps if link times become annoying.
- **Add more FHIR versions:** rebuild with
  `cargo run -p helios-hfs --features R4,R4B,R5,R6`. R6 downloads specs from
  `build.fhir.org` at build time; keep the proxy cleared.
- **Switch to PostgreSQL:** set `HFS_STORAGE_BACKEND=postgres` and
  `HFS_DATABASE_URL="postgresql://user:pass@host:5432/fhir"`.
- **Run the sibling terminology server (HTS):** the same build produced
  `target\debug\hts.exe`. Launch on port 8090, then set
  `HFS_TERMINOLOGY_SERVER=http://127.0.0.1:8090` on HFS to enable `:in`
  search modifiers and FHIRPath `memberOf()` / `subsumes()`. Bootstrap
  data (VSAC, PHINVADS, ICD-10-CM, HL7 packs, MeSH, NCI Thesaurus, NDC,
  UCUM, etc., ~148 MB) sits at `crates\hts\terminology-data\`.
- **UI E2E tests:** `crates\ui\e2e\` — needs Node + Playwright. Would
  need the proxy bypass for the `pnpm ci` step (same rule as this
  session).

---

## 9. Reference

- Project docs: `README.md` (root), `AGENTS.md`, `CLAUDE.md`
- HFS runtime skill: `.claude/skills/run-hfs-server/SKILL.md`
- UI skill: `.claude/skills/work-with-ui/SKILL.md`
- HTS skill: `.claude/skills/work-with-hts/SKILL.md`
- Corporate proxy bypass rule: `~/.cursor/rules/corporate-proxy-bypass.mdc`
- winlibs releases: <https://github.com/brechtsanders/winlibs_mingw/releases>
- rustup: <https://rustup.rs/>
