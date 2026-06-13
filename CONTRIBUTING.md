# Contributing to the Helios FHIR Server

Thanks for your interest in contributing! This guide covers how to get set up,
the standards we hold pull requests to, and our policies around commits.

We welcome contributions at every level — see the [Community](README.md#community)
section for ways to get involved beyond code (GitHub Discussions, issues, and the
weekly developer meeting).

## Getting Started

1. **Install [Rust](https://www.rust-lang.org/tools/install)** (minimum supported
   version: **1.90**, edition 2024).
2. Fork the repository and clone your fork.
3. Build and test to confirm your environment works:
   ```bash
   cargo build
   cargo test
   ```

See [`CLAUDE.md`](CLAUDE.md) for the full set of build, test, and run commands,
including per-crate builds, FHIR version feature flags, and server configuration.

## Signed Commits

**All commits must be signed.** We require cryptographically verified commits on
every branch that merges into `main`. This guarantees the authenticity and
integrity of the project's history, and it matches how our automated release and
CI tooling already operates (see [`release.toml`](release.toml), which sets
`sign-commit = true` / `sign-tag = true`).

Unsigned commits will be rejected by branch protection and cannot be merged.

### One-time setup

You can sign with either GPG or SSH. SSH signing is simplest if you already push
over SSH.

**Option A — SSH signing (recommended if you already use an SSH key):**
```bash
git config --global gpg.format ssh
git config --global user.signingkey ~/.ssh/id_ed25519.pub
git config --global commit.gpgsign true
git config --global tag.gpgsign true
```
Then add the **same** key as a *Signing Key* in your GitHub account
(Settings → SSH and GPG keys → New SSH key → Key type: *Signing Key*).

**Option B — GPG signing:**
```bash
# Generate a key if you don't have one
gpg --full-generate-key

# Find your key id, then configure git
git config --global user.signingkey <YOUR_KEY_ID>
git config --global commit.gpgsign true
git config --global tag.gpgsign true
```
Export the public key (`gpg --armor --export <YOUR_KEY_ID>`) and add it under
GitHub Settings → SSH and GPG keys → New GPG key.

> Make sure the email on your signing key matches the email in your Git config
> and a verified email on your GitHub account, or GitHub will show the commit as
> "Unverified".

### Signing individual commits

With `commit.gpgsign true` set, every commit is signed automatically. To sign a
single commit explicitly:
```bash
git commit -S -m "your message"
```

To verify your commits are signed:
```bash
git log --show-signature -1
```

### Fixing unsigned commits

If you've already made unsigned commits on your branch, re-sign them with:
```bash
# Re-sign the last N commits (replace N)
git rebase --exec 'git commit --amend --no-edit -S' -i HEAD~N
```

## Pull Requests

1. Create a topic branch off `main` (e.g. `feat/...`, `fix/...`, `docs/...`).
2. Keep changes focused; a PR should address one logical concern.
3. Before opening a PR, run the standard pre-merge checks (see below).
4. Open the PR against `main` and fill in a clear description of what and why.

### Pre-merge checks

Run these before pushing — they mirror what CI enforces:

```bash
# Format all code
cargo fmt --all

# Lint with the CI-compatible flags
cargo clippy --all-targets --all-features -- -D warnings \
  -A clippy::items_after_test_module \
  -A clippy::large_enum_variant \
  -A clippy::question_mark \
  -A clippy::collapsible_match \
  -A clippy::collapsible_if \
  -A clippy::field_reassign_with_default \
  -A clippy::doc-overindented-list-items \
  -A clippy::doc-lazy-continuation

# Run the affected crates' tests
cargo test
```

### Commit messages

We follow [Conventional Commits](https://www.conventionalcommits.org/)
(`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `ci:`, `chore:`). Scope the
subject where helpful, e.g. `feat(search): ...`.

When a change only touches documentation or other non-compiled files, include
`[skip ci]` in the commit message to avoid unnecessary CI builds.

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE.md) that covers this project.
