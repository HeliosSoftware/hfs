# Book page conventions

How pages under `book/src/` are written. The two reference pages are
`book/src/components/web-ui-self-calls.md` and
`book/src/components/natural-language-search.md`. Open one before writing;
every excerpt below is quoted from them. Rules marked "new pages" are
requirements for pages you add; the exemplars do not exercise them.

## Structure

- Exactly one `#` heading, in title case, equal to the page's text in
  `book/src/SUMMARY.md`. Example: `# Natural-Language Search` and the line
  `- [Natural-Language Search](components/natural-language-search.md)`.
- `##` and `###` headings in sentence case: `## Degraded state`,
  `### Turning it on`, `## Getting an API key`. Em dashes are fine inside a
  heading (`## What it does — and what it does not`).
- The first paragraph says what the feature is and why it matters, with the
  key terms in bold:

  > The feature is **off unless you configure it**, and an operator can make
  > it disappear entirely.

- Keep one subject per page. A page may link to others; it does not retell them.

## Voice

- Second person and imperative: "Leave the token's **tenant claim unset** for
  service tokens", "Then open `/ui/search`."
- State limits openly, in prose, next to the thing they limit. Do not hide a
  caveat in a footnote. Example heading: `## What it does — and what it does not`.
- Bold marks the decisive word of a sentence, not whole sentences.
- No admonition blocks (`> [!NOTE]` and the like), no images, no diagrams.

## Line width

Prose is wrapped at 80 columns. Tables, fenced code and URLs are exempt. The
exemplars wrap every prose paragraph by hand; match that.

## Tables

- Configuration tables use this header, then one row per variable:

  ```text
  | Variable | Default | Description |
  |----------|---------|-------------|
  | `HFS_NL_SEARCH_API_KEY` | *(unset)* | LLM provider API key. ... |
  ```

- A variable with no default shows `*(unset)*`, never an empty cell.
- Mode or state matrices put a bold label in column 1:

  ```text
  | `true` | unset | **Advertised.** The Search page explains the feature ... |
  | **Off** (default) | `HFS_AUTH_ENABLED` unset or `false` | ... |
  ```

- Every table row is a single physical line; never wrap a row.
- Defaults come from the code (see `checklist.md`), not from another doc.

## Code blocks

- Every opening fence carries a language tag: `bash`, `json` or `text`. A bare
  opening fence is a defect.
- Shell examples use `export` for configuration:

  ```bash
  export HFS_NL_SEARCH_API_KEY=sk-ant-...
  cargo run -p helios-hfs --bin hfs
  ```

- Break long commands with a trailing `\` and indent the continuation by two
  spaces:

  ```bash
  curl -X POST http://localhost:8080/\$nl-search \
    -H 'Content-Type: application/json' \
  ```

- Do not prefix commands with `$` or `>`; readers copy these blocks.
- Show only commands that were actually run, with the output actually seen.
  Anything else is described in prose as **not verified** in the limits
  section (see `page-template.md`).

## Placeholders (new pages)

Write values the reader must replace in braces, lower-case with hyphens:
`{domain}`, `{auth-server-id}`, `{client-id}`, `{HFS_BASE_URL}`. Use them in
URLs and commands, and say once what each one is. Do not paste real
identifiers, tokens or tenant names. Never write a literal `{{`: mdBook treats
it as a template sequence. (The exemplar `web-ui-self-calls.md` writes
`<token>` inside a table cell; braces are the convention for new pages.)

## Links

- Link to other book pages with relative `.md` paths, for example
  `../ch03-quickstart.md` from a page in `components/`.
- Link only to pages listed in `SUMMARY.md`. An unlisted page is not rendered
  and returns 404 on the published site.
- External links are plain Markdown links:
  `[console.anthropic.com](https://console.anthropic.com/settings/keys)`.
- Repository paths are inline code, not links:
  `crates/rest/src/handlers/nl_search_prompt.md`.
- Check anchors against real headings; run `scripts/check-book-links.sh`.

## Environment variables

Name them in backticks. Group the defaults in one `## Configuration` table.
Explain combinations in a small matrix (as in the three-state table of
`natural-language-search.md`) instead of a paragraph of "if A and B".

## Adding the page to the book

Add exactly one line to `book/src/SUMMARY.md`, in the style of its neighbours,
and leave every other line unchanged. Do not edit `configuration/`,
`getting-started/` or `development/` pages.
