# History & Versions: diff rendering (decision record)

**Status:** Decided — see [#236](https://github.com/HeliosSoftware/hfs/issues/236) for the
full research, alternatives table, and discussion.
**Scope:** how the History & Versions screen (`docs/ui-requirements.md` §5)
renders the difference between two versions of a resource from `_history`.

## Decision

Render the diff **server-side, in two layers**, into Askama templates; htmx
swaps the fragment when the version selectors change, and the base view works
with JavaScript off (plain form submission). No client-side diff library.

1. **Semantic layer — `json_patch::diff`.** Already a default-feature
   dependency of `helios-rest` and `helios-persistence` (3.0.1 in
   `Cargo.lock`), so this adds **zero dependencies**, and it is the exact
   inverse of the RFC 6902 patches `crates/rest/src/handlers/patch.rs`
   already applies for the FHIR `PATCH` interaction. Rendered as a
   field-level change list (`replace /name/0/family Smith → Smythe`).
2. **Textual layer — [`similar`](https://docs.rs/similar)** (Apache-2.0, no
   default deps, actively maintained) with the `inline` feature for
   word-level intra-line highlighting over the pretty-printed JSON of both
   versions. `similar` has no HTML formatter; we iterate
   `Change`/`InlineChange` and emit rows from an Askama template — which is
   what the crate's no-inline-markup rule wants anyway. The diff-table CSS is
   ours to write in `app.css`; crib diff2html's visual language
   (side-by-side, paired lines, +/- gutters). The dependency is added when
   the screen is built, not before.

diff2html, jsondiffpatch, Monaco/CodeMirror merge views, and `imara-diff`
were evaluated and rejected (CDN/vendored-JS weight, JS-off blank pane, or
no word-level API) — the comparison table lives in the issue.

## Spike result: RFC 6902 array churn, characterized

`json_patch::diff` over two versions of a Patient where v4 renames a family,
**inserts a new name at the front of `name[]`**, and appends a telecom:

```
replace /meta/lastUpdated  → 2026-07-09T12:01:00Z
replace /meta/versionId    → 4
replace /name/0/family     → Smythe
replace /name/0/given/0    → Johanna
add     /name/0/use        → official
replace /name/1/given/0    → Jon
remove  /name/1/use
add     /name/2            → {family: Smith, given: [J], use: nickname}
add     /telecom/1         → {system: email, value: j@x.org}
```

The front-insert renders as a six-op cascade across `name/0..2` instead of a
single `add /name/0`; the clean append (`telecom/1`) is one op.

## Design decisions

- **Array churn:** accepted for v1. Move detection is a presentation-layer
  post-process that can be added later without changing the architecture;
  the textual layer already shows an insert legibly.
- **Noise floor:** `meta.versionId` / `meta.lastUpdated` change on every
  version — filtered from the semantic view by default, behind a "show
  metadata changes" toggle. Verified in the spike: stripping `meta` before
  `diff` removes exactly those two ops.
- **Version selection:** adjacent by default (`n` vs `n-1`); two `<select>`s
  posting a plain form, upgraded by htmx to a fragment swap.
- **Where computed:** inside the `helios-ui` handler via two `vread`s. No
  new FHIR-facing endpoint — there is no standard `$diff` operation, and a
  Helios extension is not warranted for a UI concern.
- **Deleted versions:** a deleted endpoint renders as a state banner
  (`deleted`) with the surviving side shown whole as added/removed content —
  no semantic diff against a tombstone.
