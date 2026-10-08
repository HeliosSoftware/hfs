# Pre-merge checklist

Run every check from the repository root. Set the page first (use several
paths separated by spaces where a command takes `$PAGES`):

```bash
PAGE=book/src/components/my-page.md
PAGES="$PAGE"
```

The two exemplar pages, `book/src/components/web-ui-self-calls.md` and
`book/src/components/natural-language-search.md`, are the baseline: run any
command on them to see what a clean result looks like. The exemplars are not
perfect. The width check reports one genuine 83-character line (line 99) in
`natural-language-search.md`; do not treat that as a false positive in your
own page.

## 1. Environment variables exist in the code

List every `HFS_*` name the page uses and count the Rust files that read it as
a string literal. Any line printed with a count of `0` is a name the code does
not know. Fix the page (or confirm the variable is read some other way).

```bash
for v in $(grep -ohP 'HFS_[A-Z0-9]+(_[A-Z0-9]+)*(?![A-Z0-9_])' $PAGE | sort -u); do
  echo "$v $(grep -rlF "\"$v\"" crates --include=*.rs | wc -l)"
done | awk '$2==0'
```

Empty output is a pass. Prefix mentions such as `HFS_UI_*` are skipped by the
pattern. Then read the code for each default you wrote, for example:

```bash
grep -rn '"HFS_AUTH_\|"HFS_SMART_\|"HFS_UI_LOGIN_\|"HFS_BASE_URL"' crates --include=*.rs
```

Take defaults from that code, never from a README.

## 2. Prose width (80 columns)

Prints `line-number<TAB>text` for every prose line longer than 80 characters.
Tables, fenced code and URL-only lines are exempt (tables and fences are
filtered out; check long URL lines by eye). The pipe through
`LC_ALL=C.UTF-8` makes the count characters instead of bytes, so an em dash
counts as one column.

```bash
awk '/^```/{c=!c; next} !c && !/^\|/ {print NR "\t" $0}' $PAGE \
  | LC_ALL=C.UTF-8 grep -P '^\d+\t.{81,}$'
```

Do not use `awk 'length>80'` on its own: the default `awk` here counts bytes
and flags lines that contain an em dash even though they fit.

## 3. Fence tags

Untagged opening fences (must print nothing):

```bash
awk '/^```/{n++; if(n%2 && $0=="```") print FILENAME ":" FNR}' $PAGE
```

Opening fences with a tag other than `bash`, `json` or `text` (must print
nothing):

```bash
awk '/^```/{n++; if(n%2 && $0!~/^```(bash|json|text)$/) print FILENAME ":" FNR ": " $0}' $PAGE
```

Also make sure every block is closed: the number of lines starting with three
backticks must be even.

## 4. Secrets and identifiers

Must find nothing: no JWT, no client or server identifier, no tenant host, no
SSWS token.

```bash
grep -nE 'eyJ[A-Za-z0-9_-]{20,}|0oa[A-Za-z0-9]{8,}|aus[A-Za-z0-9]{8,}|dev-[0-9]+\.okta|SSWS ' $PAGE
```

The `0oa`, `aus`, `dev-N.okta` and `SSWS` patterns are Okta's ID and token
formats; when documenting another identity provider, add that provider's formats.

Also reread every `export` line and URL by eye: real passwords, API keys and
client secrets must be placeholders (`sk-ant-...`, `{client-id}`).

## 5. Links and SUMMARY

```bash
.claude/skills/work-with-book/scripts/check-book-links.sh $PAGES
```

It fails when a link target is missing or not listed in `SUMMARY.md`, and warns
when an anchor matches no heading. Run it on changed files only: running it with no
arguments (every page) reports existing FAILs on pages you did not touch: links
from `ch02-installation.md`, `ch03-quickstart.md` and `appendix-a-cli.md` to
unlisted pages under `configuration/` and `getting-started/`. They are not part
of your change; do not fix them.

Check that `SUMMARY.md` gained exactly one line:

```bash
git diff -U0 book/src/SUMMARY.md
```

## 6. Build

Install the prebuilt mdBook 0.4.40 binary (the version CI pins); do not compile
it. Then build and confirm no page was created by `create-missing`:

```bash
mkdir -p ~/.local/bin
curl -sSL https://github.com/rust-lang/mdBook/releases/download/v0.4.40/mdbook-v0.4.40-x86_64-unknown-linux-gnu.tar.gz \
  | tar -xz --directory ~/.local/bin mdbook
```

Make sure `~/.local/bin` is on your `PATH`.

```bash
(cd book && mdbook build)
git status --short book/src
```

`mdbook build` must exit 0. `git status` must list only the page you wrote and
`SUMMARY.md`. Any other untracked `.md` under `book/src` is an empty page that
mdBook created for a path in `SUMMARY.md` that does not exist: fix the path and
delete the stray file. `book/book/` is build output and must stay untracked.

## 7. Last read

- One `#` heading, equal to the `SUMMARY.md` text.
- Every command shown was run, with the output you saw; the rest is under
  **not verified** in the limits section.
- Nothing in the page belongs to another subject.
- The page ships with the next `v*` tag, not when the pull request merges.
