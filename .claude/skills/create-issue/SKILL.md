---
name: create-issue
description: File a GitHub issue describing work to be done — and stop there. Use when the user says "open an issue", "file a bug", "create a ticket", "make an issue for X", or wants something captured for later instead of fixed now. Creates the issue only; never implements the fix.
---

# Create a GitHub Issue

**Create the issue. Do not do the work.**

This skill ends when the issue URL is printed. Writing the issue is the entire
deliverable — the fix, the refactor, the test, the doc change described in the
issue are explicitly out of scope.

## Hard rules

- Do NOT edit, create, or delete any source file.
- Do NOT run builds, tests, formatters, or linters to "verify" the problem.
- Do NOT open a branch, commit, or PR.
- Do NOT start implementing after creating the issue, even if the fix looks
  trivial or you already know exactly what it is. Put that knowledge *in the
  issue body* instead.
- If the user wants the work done too, they will ask in a separate turn. Wait
  for it.

Read-only investigation is allowed and encouraged — `grep`, `cat`, `git log`,
`gh` — but only enough to write a specific, actionable issue.

## Steps

1. **Understand the ask.** Take the user's description at face value. If the
   subject is vague enough that two different issues could be written from it,
   ask one clarifying question; otherwise proceed.

2. **Ground it in the repo (read-only).** Spend a few searches finding the
   relevant file(s), function names, and line numbers so the issue points at
   real code rather than a vague area. Cite as `path/to/file.rs:123`.

3. **Check for duplicates.**
   ```bash
   gh issue list --search "<keywords>" --state all --limit 20
   ```
   If a close match exists, show it to the user and ask whether to comment on
   it instead of opening a new one.

4. **Pick labels from what the repo actually has.**
   ```bash
   gh label list --limit 100
   ```
   Only apply labels that already exist — `gh issue create` fails on unknown
   labels. When nothing fits cleanly, apply none.

5. **Write the body** to a file in your session scratchpad directory (a
   heredoc avoids shell-quoting damage to backticks, `$`, and newlines):
   ```bash
   cat > "$TMP/issue-body.md" <<'EOF'
   ...body...
   EOF
   ```

6. **Create it.**
   ```bash
   gh issue create --title "<title>" --body-file "$TMP/issue-body.md" --label bug
   ```
   Add `--assignee @me` only if the user asked for it. Do not `--assign` others
   uninvited.

   If `gh issue create` misbehaves in this repo, fall back to the REST API:
   ```bash
   gh api repos/{owner}/{repo}/issues -f title="..." -F body=@"$TMP/issue-body.md"
   ```

7. **Report the URL and stop.** One or two sentences. Then hand control back.

## Issue shape

Title: one specific line, imperative or symptom-first. `Tenant slug mirror
drops trailing hyphen on paste`, not `UI bug`.

Body — include only the sections that carry real content:

```markdown
## Summary
One or two sentences: what is wrong or what should exist.

## Current behavior
What happens today, with the concrete trigger. Include the exact command,
request, or click path.

## Expected behavior
What should happen instead.

## Evidence
- `crates/ui/src/routes/tenants.rs:88` — the slug is trimmed before validation
- Error output, log lines, or failing assertion, in a fenced block

## Suggested approach
Optional. A sketch of the fix for whoever picks this up — this is where your
analysis goes *instead of* into a code change.

## Scope / out of scope
Optional. Useful when the obvious reading of the title is broader than intended.
```

For a feature request, swap Current/Expected for **Motivation** and
**Proposed behavior**, plus **Acceptance criteria** as a checklist.

## Style

- Write for a contributor who has not seen this conversation. No "as we
  discussed", no references to the current session.
- Be specific over exhaustive — file paths and reproduction steps beat prose.
- No emoji, no severity theater, no "🚨 CRITICAL".
- Do not attribute the issue to Claude or add tool footers unless asked.
