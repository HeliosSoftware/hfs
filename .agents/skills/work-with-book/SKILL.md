---
name: work-with-book
description: Write or edit pages of the HFS mdBook documentation site under `book/` (`book/src/*.md`, `SUMMARY.md`, `book.toml`). Use for new book chapters and how-to guides (for example identity-provider setup), configuration tables of `HFS_*` variables, cross-links, local mdBook preview/build, and how the book is published to GitHub Pages. Not for crate READMEs or notes under `docs/`.
---

# HFS documentation book (`book/`)

The book is the mdBook source of the HFS documentation site, published at
https://heliossoftware.github.io/hfs/. Pages are Markdown files under
`book/src/`; `book/src/SUMMARY.md` is the table of contents and decides which
pages are rendered. This skill is the short form; the detail is in `references/`.

## Facts

- mdBook is pinned to 0.4.40 in `.github/workflows/ci.yml:1856`, installed from a
  prebuilt binary (`ci.yml:1856-1857`). Nothing is compiled; do the same locally.
- `book/book.toml` enables only the `links` preprocessor (line 12).
  `create-missing` is not set, so it keeps its default (true): a path listed in
  `SUMMARY.md` that does not exist is silently created as an empty page.
- The book is built and published only by the `publish-report` job, which runs
  on `v*` tags (`ci.yml:1843`: `if: startsWith(github.ref, 'refs/tags/v')`).
  A merged page goes live with the next release tag, not at merge.
- `book/README.md:40` says the site deploys on pushes to `main`. That is wrong;
  trust `ci.yml`.
- No other job in `.github/workflows/` runs `mdbook`, so a broken book is found
  only when a release is tagged. Build it yourself before you finish.
- `book/README.md` suggests `cargo install mdbook`; prefer the prebuilt 0.4.40
  binary to avoid a long compile and a version mismatch with CI.

## Workflow

1. Gather the facts first: read the code for every setting, run every command
   you will document, and record the observed result.
2. Copy `references/page-template.md` to `book/src/<area>/<name>.md`.
3. Write the page following `references/conventions.md`.
4. Add exactly ONE line for the page to `book/src/SUMMARY.md`; leave all other
   lines unchanged. An identity-provider page goes in
   `book/src/identity-providers/`, indented under `- [Identity Providers]()`.
5. Run `scripts/check-book-links.sh <changed pages>` (relative to this skill).
6. Run the mechanical greps in `references/checklist.md` (line width, untagged
   fences, secrets).
7. Run `mdbook build` in `book/`, then `git status`. The build must exit 0 and
   must not leave new untracked files under `book/src` (that is how
   `create-missing` shows up). `book/book/` is git-ignored.

Preview while writing with `mdbook serve` in `book/` (http://localhost:3000).

## Content rules

- Take environment variables and defaults from the code, never from a README:
  `grep -rn '"HFS_AUTH_\|"HFS_SMART_\|"HFS_UI_LOGIN_\|"HFS_BASE_URL"' crates --include=*.rs`.
- Document only commands that were actually run, each with its observed result.
  Label everything else **not verified** in prose, in the limits section.
- Write only about the page's subject; do not import unrelated findings.
- Use placeholders such as `{domain}`, `{client-id}`, `{HFS_BASE_URL}`; never
  paste real tokens, client IDs or tenant hostnames.
- Link with relative `.md` paths, and only to pages listed in `SUMMARY.md`.
- Wrap prose at 80 columns (tables, code and URLs exempt). Tag every opening
  code fence (`bash`, `json` or `text`).

## Do not

- Edit `configuration/`, `getting-started/`, `development/` or other pages that
  are not listed in `SUMMARY.md`.
- Use admonitions, images, diagrams, `{{` sequences (the preprocessor reads
  them) or `$` prompts in code blocks.
- Fix unrelated broken links while writing a page; run the link script on your
  changed files only.

## References

- `references/conventions.md`: structure, voice, tables, code blocks, links.
- `references/page-template.md`: skeleton for a new how-to page.
- `references/checklist.md`: pre-merge checks as copy-paste commands.
- `scripts/check-book-links.sh`: read-only check of `SUMMARY.md` paths and the
  links in the pages you pass.
