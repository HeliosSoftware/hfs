---
name: create-github-issue
description: Create a GitHub issue in this repo from a short description of a problem or request. Use when the user says "create a github issue", "file an issue", "open a ticket", or describes a bug/feature and asks for it to be tracked rather than fixed. Writes the issue only — never implements the fix.
---

# Create a GitHub issue

**Create a github issue. Don't do the work - only create the issue.**

The deliverable is the issue itself. Do not implement the fix, do not edit source
files, do not open a branch or PR. Investigate only as far as needed to make the
issue accurate and actionable.

## Steps

1. **Confirm the repo.**

   ```bash
   gh repo view --json nameWithOwner -q .nameWithOwner
   ```

   In a worktree, run everything from the worktree directory.

2. **Ground the issue in the code (lightly).** Spend a few searches locating the
   relevant file(s) so the issue can name them and quote the offending snippet.
   Read enough to be correct; stop before you start designing the fix.

3. **Resolve assignees, if asked.** The user usually gives a first name. Map it to
   a GitHub login:

   ```bash
   gh api repos/<owner>/<repo>/collaborators --jq '.[].login'
   gh api users/<login> --jq '.name'
   ```

   If two collaborators could plausibly match the name, ask the user rather than
   guessing.

4. **Check labels before using any.** Only pass `--label` with labels that exist:

   ```bash
   gh label list --repo <owner>/<repo>
   ```

   A nonexistent label makes `gh issue create` fail outright.

5. **Write the body to a file, then create.** Heredoc into the scratchpad
   directory and pass `--body-file` — this avoids shell-quoting damage to
   backticks, code fences, and newlines.

   ```bash
   cat > "$SCRATCH/issue.md" <<'BODY'
   ...
   BODY

   gh issue create \
     --repo <owner>/<repo> \
     --title "<area>: <short, specific problem statement>" \
     --body-file "$SCRATCH/issue.md" \
     --assignee <login>
   ```

6. **Report the URL** that `gh issue create` prints.

## Issue body shape

Keep it tight — a reader should understand the problem without opening the code.

- **Problem** — what happens now, and why it's wrong from the user's point of view.
- **Where** — file paths (with line numbers where useful) and a short quoted
  snippet of the actual offending code.
- **Expected behavior** — what should happen instead, as a short list. Describe
  outcomes and constraints, not an implementation plan.
- **Notes** — related places worth checking, project conventions the fix must
  respect (e.g. new user-facing strings go through the i18n locale files), and
  anything deliberately left out of scope.

## Title conventions

`<area>: <problem>` — e.g. `UI: no progress indication while adding a tenant`.
State the defect, not the fix. Lowercase after the colon, no trailing period.

## Don'ts

- Don't write the patch, even a "small obvious" one.
- Don't add speculative scope the user didn't ask for.
- Don't invent labels, milestones, or assignees.
- Don't attach the Claude session footer to issues — that convention is for
  commits and PR bodies.
