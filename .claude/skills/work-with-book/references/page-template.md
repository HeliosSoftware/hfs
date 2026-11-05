# Page template

Copy the block below into `book/src/<area>/<page>.md` and fill it in. Delete
any section that does not apply to the page, but keep the order. Read
`conventions.md` first; run `checklist.md` before you finish.

```text
# Page Title In Title Case

One paragraph, wrapped at 80 columns: what this page covers and why the
reader needs it. Put the **key terms** in bold. Say what the page does not
cover and link to the page that does.

## Prerequisites

- What must be running or installed, with the version where it matters.
- What the reader needs to hold before starting (an account, a key, a URL).
- Values the reader replaces are written in braces: {client-id}, {domain}.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_EXAMPLE_VARIABLE` | *(unset)* | What it controls and when to set it. |

Explain combinations of variables in a second table with bold labels in
column 1, one physical line per row.

## Verify

Say what a successful run looks like in one sentence, then show the commands
you actually ran.

    ```bash
    export HFS_EXAMPLE_VARIABLE=value
    curl -s {HFS_BASE_URL}/metadata
    ```

Show the observed result in a tagged block (`json` or `text`).

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| What the reader sees, quoted exactly. | Why it happens. | What to change. |

## Limits and what is not verified

- State what this page does not cover.
- List every command or setting that was **not verified**, in prose, and why.
```

Notes for the copy:

- In the real page the `bash` fence under `## Verify` starts at column 0. It is
  indented four spaces above only so that this example can sit inside one
  `text` fence. Remove the indent when you copy it.
- The single `#` heading must equal the page's text in `SUMMARY.md`.
- Add the matching line to `SUMMARY.md`; a page that is not listed is not
  rendered.
- A step you could not run belongs in the last section as **not verified**,
  never in `## Verify`.
