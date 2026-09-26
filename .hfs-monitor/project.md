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
`CLAUDE.md`**. That order is also what fixes **polarity**: where a rule below
says *do this* or *never that*, it binds, even when the section it cites reads
as advice. This file does not restate the Description layer — read `AGENTS.md`
(Codex) or `CLAUDE.md` (Claude Code) for the architecture, the crate table, and
the design patterns.

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

### A rule is not a note

The two habits this file lives by look like they collide. The Derived section
says: never write the result here, cite the command. `CLAUDE.md` says of its
Windows and worktree notes: *"This is the only place these live — don't
duplicate them elsewhere; cite this section instead."* And yet the board has to
put something in a prompt.

They collide only if *rule* and *note* are the same thing. A **note** explains:
what breaks, why, which issue it cost. A **rule** is one line of instruction
with its polarity said out loud — *do this*, *never that*. What a prompt loses
when the board cites only a section title is not the knowledge (the agent's CLI
loads `CLAUDE.md` in every worktree session anyway) but the polarity: a prompt
that never names the trap cannot warn against it, and an agent reading a note
as background is free to read it as optional.

So each rule below is **one line: a verb, an explicit polarity, and a pointer
to the section that keeps the why.** That *is* citing, in the sense
`CLAUDE.md` asks for — what it forbids duplicating is the explanation, and none
is duplicated here. Anything longer than the line belongs to the cited section,
and if the two ever disagree, the precedence table above says this file wins on
the instruction and the cited section wins on the reasoning.

### Rules

- **Format the workspace from the main checkout — never run `cargo fmt --all`
  from a worktree, where it fails:** see `CLAUDE.md` → "Windows and worktree
  notes". It is a pre-merge check and CI enforces it as
  `cargo fmt --all -- --check` (`CONTRIBUTING.md` → "Pre-merge checks").
- **When verifying a change, format per crate: run `cargo fmt -p <crate>` for
  each crate you touched** — the form that works from a worktree.
  `cargo fmt -- <path>...` (`CLAUDE.md` → "Windows and worktree notes") is
  equally valid; use whichever names your change more exactly.
- **Verify in debug; do not verify with `--release`:** `cargo check`,
  `cargo test`, `cargo clippy` and `cargo build` without `--release`, which is
  what CI gates on (`.github/workflows/ci.yml`; `--release` appears there only
  in the release-artifact and benchmark jobs). This governs *verifying your
  change* — release builds remain right for shipping binaries and for anyone
  building a server to run, so "not `--release`" is not "never `--release`".
- **Use the repository's own skills when the task touches what they cover**
  (`/work-with-ui`, `/work-with-hts`, `/test-hfs`, …) instead of rediscovering
  it: the full list is in `CLAUDE.md` → "Project Skills" (Claude Code, under
  `.claude/skills/`) and `AGENTS.md` → "Project Skills" (Codex, under
  `.agents/skills/`).
- **Discard output with `/dev/null`, never `> nul`:** the shell here is bash on
  Windows too, so `> nul` writes a real file and leaves the worktree
  unremovable ever after (#942, #969) — see `CLAUDE.md` → "Windows and worktree
  notes".
- **Stage explicit paths — never `git add -A` or `git commit -a` in a worktree
  here:** compiling leaves ~3,500 R6 spec files `stat`-dirty even when nothing
  was edited (`CLAUDE.md` → "Windows and worktree notes").

### Also

- **Other build/test commands:** `CONTRIBUTING.md` → "Pre-merge checks" has the
  clippy invocation with its allow-list and the `cargo test` run; `CLAUDE.md` →
  "Environment Setup" has linker and memory-constrained build settings.
- **Windows/worktree traps beyond the rules above:** `CLAUDE.md` → "Windows and
  worktree notes" is the only place they live; do not let a second copy drift
  out of sync with it.
- **Commit style:** [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`), see
  `CONTRIBUTING.md` → "Commit messages". Every commit on a branch that merges
  into `main` must be signed (`CONTRIBUTING.md` → "Signed Commits"). Topic
  branches off `main`: `feat/...`, `fix/...`, `docs/...`. A documentation-only
  change carries `[skip ci]` (`CONTRIBUTING.md` → "Commit messages").
