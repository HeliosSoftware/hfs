# hfs — project context for hfs-monitor

This file is read by the hfs-monitor board from hfs's **main checkout**
(`project.repoPath`), never from a task's worktree — the agent working a task
does not get to choose the context it is evaluated against. It supplies two
of the three context layers the board composes into a task prompt:

| Layer | Source | Conflict? |
|---|---|---|
| **Derived** — crates, binaries, MSRV | a command declared below, run by the board | Impossible: it *is* the repository |
| **Conventions** — build/test commands, traps, commit style | the prose below | Single source |
| **Description** — architecture, design patterns | `AGENTS.md` / `CLAUDE.md` | Between those two; not this file |

Precedence when they disagree: **derived › conventions › `AGENTS.md` ›
`CLAUDE.md`**. This file does not restate the Description layer — read
`AGENTS.md` (Codex) or `CLAUDE.md` (Claude Code) for the architecture, the
crate table, and the design patterns.

## Derived

These are commands to run against the checkout, not numbers to copy. Writing
the *result* here as fixed text is exactly the mistake this file exists to
prevent — `AGENTS.md` said 17 crates and `CLAUDE.md` said 20 while the
workspace had grown to 22; both had drifted silently. Run the command, use
its output.

- **Crate count, total:** count directories under `crates/` that contain
  their own `Cargo.toml` (e.g. `ls crates/*/Cargo.toml | wc -l` from a POSIX
  shell, or an equivalent directory listing).
- **Crate count, default build:** count the entries under `[workspace]` →
  `default-members` in the root `Cargo.toml`.
- **Full crate graph, machine-readable:** `cargo metadata --no-deps --offline`
  run from the repository root. This only reads manifests — it does not
  compile anything, so it is safe to run from a worktree without triggering
  the R6-spec trap below.
- **MSRV:** the `rust-version` key under `[workspace.package]` in the root
  `Cargo.toml` (or `rust-toolchain.toml`, if one is added later).

## Conventions

- **Build/test commands:** see `CLAUDE.md` → "Environment Setup" and
  `CONTRIBUTING.md` → "Pre-merge checks" (`cargo fmt --all`,
  `cargo clippy --all-targets --all-features -- -D warnings` with the
  documented allow-list, `cargo test`).
- **Windows/worktree traps:** see `CLAUDE.md` → "Windows and worktree notes".
  Cited here, not copied — that section is the only place they live; do not
  let a second copy drift out of sync with it.
- **Commit style:** [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`), see
  `CONTRIBUTING.md` → "Commit messages". Every commit on a branch that merges
  into `main` must be signed (`CONTRIBUTING.md` → "Signed Commits"). Topic
  branches off `main`: `feat/...`, `fix/...`, `docs/...`.
- **Staging:** never `git add -A` or `git commit -a` in a worktree here —
  compiling leaves ~3,500 R6 spec files dirty in `stat` even when nothing was
  edited. Stage explicit paths.
