//! Shared chrome markup for the two Helios web UIs.
//!
//! # Why this crate exists
//!
//! HFS (`crates/ui`) and HTS (`crates/hts-ui`) present one product to an
//! operator, so their topbar has to look and behave the same. Until now that
//! was maintained by copying markup between the two template trees and hoping.
//! It did not hold:
//!
//! * `crates/hts-ui/templates/layouts/base.html` carried a comment asserting
//!   the account menu was byte-for-byte the HFS block. That assertion silently
//!   stopped being true.
//! * A `.lang-switcher` control survived in the HTS copy after HFS had deleted
//!   both the control and its CSS rule — HTS was rendering markup that nothing
//!   styled any more.
//! * `crates/hts-ui/tests/chrome_parity.rs` grew to 517 lines whose entire job
//!   is to notice that kind of drift *after* it has been committed.
//!
//! A test that detects divergence is strictly worse than a structure that
//! cannot diverge. So the markup lives here, once, and both crates render it
//! through [`user_menu`]. There is no second copy to fall out of step with.
//!
//! # What is shared, and how
//!
//! The template is [`templates/partials/user-menu.html`][partial] plus the two
//! icons it includes. `{% include "icons/..." %}` resolves against *this*
//! crate's `templates/` root, because the `#[derive(Template)]` that owns the
//! partial lives in this crate — a consumer needs no icon files of its own.
//!
//! [partial]: https://github.com/HeliosSoftware/hfs/blob/main/crates/ui-chrome/templates/partials/user-menu.html
//!
//! The partial is a byte-verbatim extract of what used to be
//! `crates/ui/templates/layouts/base.html:233-267`, so moving it changed no
//! HFS page byte. Preserving that is a hard requirement, and it is what the
//! trailing whitespace-suppression sentinel in the partial is for: the caller
//! supplies the newline after `</details>`, so the render must not end with
//! one. See `tests/user_menu.rs::renders_without_a_trailing_newline`.
//!
//! The second thing shared here is the JSON viewer — [`json_view`] — for the
//! same reason. It arrived in #803, when the HTS workbench's raw request and
//! response fold needed the line-numbered, foldable, highlighted view HFS has
//! rendered since #264, and the only alternatives were depending on the whole
//! HFS UI crate or forking a 400-line tokenizer.
//!
//! # What is deliberately *not* shared here
//!
//! CSS. `crates/ui/assets/app.css` is already shared byte-for-byte by
//! `crates/hts-ui/src/lib.rs` embedding `../ui/assets` directly, and lifting
//! it into a neutral crate is gated on #543. The same goes for
//! `json-view.js`, which drives the folding for both products from that embed.
//!
//! # Usage
//!
//! ```no_run
//! use helios_ui_chrome::{ChromeLabels, UserIdentity, user_menu};
//!
//! struct Labels;
//! impl ChromeLabels for Labels {
//!     fn lang(&self) -> String {
//!         "en".to_string()
//!     }
//!     fn t(&self, key: &str) -> String {
//!         // a real consumer routes this into its fluent bundle
//!         key.to_string()
//!     }
//! }
//!
//! let html = user_menu(&Labels, UserIdentity::default())?;
//! # Ok::<(), askama::Error>(())
//! ```
//!
//! The rendered fragment is normally handed to the consuming layout as a
//! pre-escaped string, e.g. `{{ user_menu|safe }}` — see the note on
//! [`user_menu`] about why that is sound.

use askama::Template;

pub mod json_view;

/// The localisation surface the shared chrome needs from its host.
///
/// Both products already own a fluent-backed i18n type; this trait is the
/// narrow slice of it the chrome actually uses, so neither crate has to expose
/// its bundle machinery here and this crate depends on no i18n library at all.
///
/// Implementations are looked up dynamically ([`user_menu`] takes
/// `&dyn ChromeLabels`), which keeps the template a single monomorphisation
/// shared by both binaries.
pub trait ChromeLabels {
    /// The BCP-47 language tag currently in effect, e.g. `"en"`, `"es"`, `"de"`.
    ///
    /// The language segmented control marks exactly the matching entry with
    /// `aria-current="true"`. A tag this crate does not offer simply leaves all
    /// three unmarked; that is a valid state, not an error.
    fn lang(&self) -> String;

    /// Resolve a message key to display text in [`lang`](ChromeLabels::lang).
    ///
    /// The keys the chrome asks for are `user-menu-label`, `user-anonymous`,
    /// `user-local-hint`, `language-label`, `language-en`, `language-es`,
    /// `language-de`, and `user-logout`, plus `json-view-toggle-fold` for
    /// [`json_view::render`]. A consumer missing any of them will render
    /// whatever its bundle's fallback produces.
    ///
    /// The returned text is HTML-escaped by the template. Do not pre-escape it,
    /// and do not return markup expecting it to render as markup.
    fn t(&self, key: &str) -> String;
}

/// Who the account menu should say is signed in.
///
/// Every field is optional or defaults to the signed-out shape, so
/// `UserIdentity::default()` renders the anonymous local-operator state that
/// both products show today (neither has a browser login flow yet — #320).
///
/// The avatar follows a three-step fallback: [`photo`](Self::photo), else
/// [`initials`](Self::initials), else a generic user icon.
#[derive(Clone, Copy, Debug, Default)]
pub struct UserIdentity<'a> {
    /// Primary line of the account card — a display name. Falls back to the
    /// `user-anonymous` label.
    pub display: Option<&'a str>,

    /// Secondary line — typically an email address or IdP subject. Falls back
    /// to the `user-local-hint` label.
    pub secondary: Option<&'a str>,

    /// One or two letters drawn in the avatar when there is no
    /// [`photo`](Self::photo).
    pub initials: Option<&'a str>,

    /// URL of an avatar image, used in preference to
    /// [`initials`](Self::initials). Emitted into `src`, so it must be a URL
    /// the consumer trusts.
    pub photo: Option<&'a str>,

    /// Whether to render the Sign out link at all. False renders no link,
    /// whatever [`logout_href`](Self::logout_href) holds.
    pub can_logout: bool,

    /// Where Sign out points. Only read when
    /// [`can_logout`](Self::can_logout) is true, which is why the default
    /// empty string is harmless.
    pub logout_href: &'a str,
}

/// The shared account menu: avatar `<summary>`, account card, language
/// segmented control, and the sign-out link.
///
/// Returns an HTML fragment indented to sit inside a `.topbar__tools`
/// container, starting at column 8 and ending at `</details>` with **no**
/// trailing newline — the caller supplies that. Splicing the result into a
/// layout with `|safe` is sound because every value that reaches the output
/// goes through the default HTML escaper first; see
/// `tests/user_menu.rs::labels_are_html_escaped_before_the_safe_filter`, which
/// exists to keep that true if an IdP claim ever lands in
/// [`UserIdentity::display`].
///
/// # Errors
///
/// Propagates [`askama::Error`] from rendering. In practice the template has
/// no fallible construct, so this is a formality the signature keeps honest.
pub fn user_menu(i18n: &dyn ChromeLabels, user: UserIdentity<'_>) -> Result<String, askama::Error> {
    UserMenuTemplate { i18n, user }.render()
}

/// Re-exported so consumers can name [`askama::Error`] — the error type of
/// [`user_menu`] — without adding their own askama dependency, and without the
/// two crates being able to drift onto different askama versions.
pub use askama;

/// The one binding of the shared partial.
///
/// The field is named `i18n` (not `labels`) on purpose: the markup was lifted
/// out of `crates/ui` verbatim and still says `{{ i18n.t("...") }}`. Renaming
/// the field would mean rewriting the template, which would break the
/// byte-for-byte extraction this crate is built on.
#[derive(Template)]
#[template(path = "partials/user-menu.html")]
struct UserMenuTemplate<'a> {
    i18n: &'a dyn ChromeLabels,
    user: UserIdentity<'a>,
}
