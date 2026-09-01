# helios-ui-chrome

Shared chrome markup for the two Helios web UIs — `helios-ui` (HFS, `/ui`) and
`helios-hts-ui` (HTS). One Askama template, one typed contract, rendered by both
products.

Not published to crates.io (`publish = false`): it is an internal implementation
detail of those two crates, not a surface anyone else should build against.

## Why

The topbar used to be maintained by copying markup between the two template
trees. That drifted: HTS carried a comment claiming byte-for-byte parity with
the HFS block long after it had stopped being true, a `.lang-switcher` control
survived in the HTS copy after HFS had deleted both the control and its CSS
rule, and `crates/hts-ui/tests/chrome_parity.rs` grew to 517 lines whose whole
job was to spot that class of divergence after the fact.

A structure that cannot fork beats a test that detects forking. So the markup
lives here once.

## What belongs here

Chrome markup shared by both products, together with the typed contract that
feeds it:

- `templates/partials/user-menu.html` — the account menu: avatar `<summary>`,
  account card, language segmented control, sign-out link.
- `templates/icons/{user,logout}.svg` — the icons that partial includes.
  `{% include %}` resolves against *this* crate's `templates/` root, so
  consumers need no copies.
- `ChromeLabels` — the narrow i18n slice the chrome needs (`lang()`, `t(key)`).
  This crate depends on no i18n library; each product adapts its own bundle.
- `UserIdentity` — who the menu says is signed in.

The partial is a byte-verbatim extract of what was
`crates/ui/templates/layouts/base.html:233-267`, so the move changed no HFS page
byte. Two things preserve that and must survive future edits:

1. The indentation (8/10/12/14 spaces) and attribute order, exactly as they are.
2. The trailing whitespace-suppressing sentinel comment. The caller supplies the
   newline after `</details>`, so the render must not end with one.
   `tests/user_menu.rs::renders_without_a_trailing_newline` fails if it goes.

`crates/ui-chrome/templates/**` and `crates/ui-chrome/tests/golden/**` carry
`text eol=lf` in `.gitattributes`. Template bytes are compiled into the binary
and compared against LF literals in tests; a CRLF Windows checkout breaks every
multi-line assertion (#671).

## What does not belong here

**CSS.** `crates/ui/assets/app.css` is already shared byte-for-byte:
`crates/hts-ui/src/lib.rs` embeds the sibling directory directly with
`#[folder = "../ui/assets"]`. Lifting the asset tree into a neutral crate is
gated on #543; until then, adding a second copy of any stylesheet here would
re-create exactly the duplication this crate exists to remove.

## Next candidate

The 12 SVGs under `crates/hts-ui/templates/icons/` are byte-identical, as
committed, to their namesakes in `crates/ui/templates/icons/`: `book`,
`bookmark`, `check`, `chevron-down`, `hierarchy`, `home`, `import`, `moon`,
`shield`, `sliders`, `sun`, `sync`. Moving them here would delete 12 duplicate
files and give both products one icon set.

Worth noting while doing it: `.gitattributes` pins `crates/ui/templates/**` to
LF but says nothing about `crates/hts-ui/templates/**`, so on a Windows checkout
with `core.autocrlf=true` the HTS copies come back with CRLF and the two trees
compare as different on disk while being identical in git. Relocating them under
`crates/ui-chrome/templates/**` puts them inside the LF rule and makes that
mismatch impossible.
