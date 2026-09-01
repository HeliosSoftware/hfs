//! Locale negotiation and Fluent catalog lookup for the HTS administrative UI.
//!
//! One locale is negotiated per request and carried in request extensions as
//! [`RequestLocale`]; templates resolve catalog keys against it through the
//! [`I18n`] helper. Policy mirrors `crates/ui/src/i18n.rs`: explicit `?lang=`
//! override (persisted in the `hts_lang` cookie by the language switcher) →
//! cookie → `Accept-Language` (RFC 4647 Lookup) → `en`.
//!
//! Catalogs live in `locales/<locale>/main.ftl` at the workspace root and are
//! embedded at compile time — no runtime file or CDN dependency. HTS-specific
//! keys carry the `hts-*` namespace (see design doc §7 Fluent convention).
//! `en` is the source locale and the final fallback: a key missing from a
//! translation renders its English string, never a blank or a crash.
//!
//! Cookie name is `hts_lang` (distinct from HFS's `hfs_lang`) so that when
//! both binaries share a domain their language preferences do not collide.

use axum::{
    extract::{FromRequestParts, Request},
    http::{HeaderMap, Uri, header, request::Parts},
    middleware::Next,
    response::Response,
};
use fluent_templates::{Loader, fluent_bundle::FluentValue};
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use unic_langid::{LanguageIdentifier, langid};

fluent_templates::static_loader! {
    static LOCALES = {
        locales: "../../locales",
        fallback_language: "en",
        // The UI renders whole localized sentences into an LTR document; the
        // Unicode bidi isolation marks Fluent adds around placeables by
        // default would only show up as garbage in tests and diffs.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

static EN: LanguageIdentifier = langid!("en");
static ES: LanguageIdentifier = langid!("es");
static DE: LanguageIdentifier = langid!("de");

/// Locales the HTS UI can render, in switcher order.
static SUPPORTED: [&LanguageIdentifier; 3] = [&EN, &ES, &DE];

/// Cookie the language switcher sets via `?lang=`. Distinct from HFS's cookie
/// so the two binaries can coexist on one domain without cross-influence.
const LANG_COOKIE: &str = "hts_lang";

/// The locale negotiated for the current request.
///
/// Inserted into request extensions by [`negotiate_locale`]; handlers take it
/// as an (infallible) extractor. Defaults to `en` when the middleware is not
/// installed, so templates always have a locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestLocale(&'static LanguageIdentifier);

impl Default for RequestLocale {
    fn default() -> Self {
        RequestLocale(&EN)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for RequestLocale {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<RequestLocale>()
            .copied()
            .unwrap_or_default())
    }
}

/// Middleware: negotiate the request locale once and expose it in request
/// extensions. When the choice came from an explicit `?lang=` (the language
/// switcher), persist it in the `hts_lang` cookie so it survives navigation.
pub async fn negotiate_locale(mut request: Request, next: Next) -> Response {
    let (locale, explicit) = negotiate(request.uri(), request.headers());
    request.extensions_mut().insert(RequestLocale(locale));

    let mut response = next.run(request).await;
    // The body varies with the negotiated inputs; without this a shared cache
    // could hand one user's language to another.
    response.headers_mut().append(
        header::VARY,
        header::HeaderValue::from_static("Accept-Language, Cookie"),
    );
    if explicit {
        let cookie = format!("{LANG_COOKIE}={locale}; Path=/; Max-Age=31536000; SameSite=Lax");
        if let Ok(value) = cookie.parse() {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

/// Resolve the locale for a request. Returns the locale and whether it came
/// from an explicit `?lang=` override (which the caller persists in a cookie).
fn negotiate(uri: &Uri, headers: &HeaderMap) -> (&'static LanguageIdentifier, bool) {
    if let Some(locale) = query_lang(uri).as_deref().and_then(match_supported) {
        return (locale, true);
    }
    let locale = cookie_lang(headers)
        .and_then(|tag| match_supported(&tag))
        .or_else(|| accept_language(headers))
        .unwrap_or(&EN);
    (locale, false)
}

/// `lang` value from the query string, if any.
fn query_lang(uri: &Uri) -> Option<String> {
    uri.query()?
        .split('&')
        .find_map(|pair| pair.strip_prefix("lang=").map(str::to_owned))
}

/// `hts_lang` cookie value, if any.
fn cookie_lang(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            pair.trim()
                .strip_prefix(LANG_COOKIE)?
                .strip_prefix('=')
                .map(str::to_owned)
        })
}

/// Best supported locale for the `Accept-Language` header, if any: ranges in
/// descending quality order, each matched per RFC 4647 §3.4 Lookup.
fn accept_language(headers: &HeaderMap) -> Option<&'static LanguageIdentifier> {
    let header = headers.get(header::ACCEPT_LANGUAGE)?.to_str().ok()?;
    let mut ranges: Vec<(&str, f32)> = header
        .split(',')
        .filter_map(|part| {
            let mut pieces = part.trim().split(';');
            let tag = pieces.next()?.trim();
            if tag.is_empty() || tag == "*" {
                return None;
            }
            let q = pieces
                .find_map(|p| p.trim().strip_prefix("q="))
                .and_then(|q| q.parse().ok())
                .unwrap_or(1.0);
            Some((tag, q))
        })
        .collect();
    ranges.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranges.iter().find_map(|(tag, _)| match_supported(tag))
}

/// RFC 4647 §3.4 Lookup of one language range against the supported set:
/// try the full tag, then progressively truncate subtags (`de-CH-1996` →
/// `de-CH` → `de`).
fn match_supported(tag: &str) -> Option<&'static LanguageIdentifier> {
    let tag = tag.trim().to_ascii_lowercase();
    let mut range = tag.as_str();
    loop {
        if let Some(locale) = SUPPORTED
            .iter()
            .find(|supported| supported.to_string().eq_ignore_ascii_case(range))
        {
            return Some(locale);
        }
        match range.rfind('-') {
            Some(idx) => range = &range[..idx],
            None => return None,
        }
    }
}

/// Template-side catalog access, bound to the request's negotiated locale.
#[derive(Clone, Copy)]
pub struct I18n {
    locale: &'static LanguageIdentifier,
}

impl I18n {
    pub fn new(locale: RequestLocale) -> Self {
        I18n { locale: locale.0 }
    }

    /// BCP 47 tag of the active locale — for `<html lang>` and the switcher.
    pub fn lang(&self) -> String {
        self.locale.to_string()
    }

    /// Look up a message. Missing keys fall back to `en`; unknown keys render
    /// the key itself rather than crashing or blanking the page.
    pub fn t(&self, key: &str) -> String {
        LOCALES
            .try_lookup(self.locale, key)
            .unwrap_or_else(|| key.to_owned())
    }

    /// Look up a message, falling back to `fallback` rather than to the key
    /// when the catalog has no entry.
    ///
    /// For keys built from server-supplied values, where the set of possible
    /// keys is open. The OperationOutcome partial composes
    /// `hts-outcome-code-<code>` from a FHIR issue code, and the catalog
    /// carries only the four the UI reasons about (`not-found`, `invalid`,
    /// `too-costly`, `unknown`). Its own comment has always promised it
    /// "falls back to the raw code so unknown codes still surface" — but
    /// [`Self::t`] returns the *key*, so an ordinary `business-rule` issue
    /// rendered the literal string `hts-outcome-code-business-rule` on the
    /// page. This makes the documented behaviour real: the reader sees
    /// `business-rule`, which is at least the truth from the server.
    pub fn t_or(&self, key: &str, fallback: &str) -> String {
        LOCALES
            .try_lookup(self.locale, key)
            .unwrap_or_else(|| fallback.to_owned())
    }

    /// Look up a message with one named placeable, e.g.
    /// `t_arg("hts-home-uptime", "duration", "3d")`.
    pub fn t_arg(&self, key: &str, name: &str, value: impl Into<FluentValue<'static>>) -> String {
        let args: HashMap<Cow<'static, str>, FluentValue<'static>> =
            HashMap::from([(Cow::Owned(name.to_owned()), value.into())]);
        LOCALES
            .try_lookup_with_args(self.locale, key, &args)
            .unwrap_or_else(|| key.to_owned())
    }

    /// Look up a message with two named placeables — used by the
    /// batch progress region (`n of m completed`) and other multi-arg
    /// formats introduced in Slice E (design doc §7.6 F1=D). The E1
    /// stub batch handlers don't render the progress partial yet, so
    /// the method is dead-code until E2 wires it in.
    #[allow(dead_code)]
    pub fn t_arg2(
        &self,
        key: &str,
        name1: &str,
        value1: impl Into<FluentValue<'static>>,
        name2: &str,
        value2: impl Into<FluentValue<'static>>,
    ) -> String {
        let args: HashMap<Cow<'static, str>, FluentValue<'static>> = HashMap::from([
            (Cow::Owned(name1.to_owned()), value1.into()),
            (Cow::Owned(name2.to_owned()), value2.into()),
        ]);
        LOCALES
            .try_lookup_with_args(self.locale, key, &args)
            .unwrap_or_else(|| key.to_owned())
    }

    /// Like [`Self::t_arg2`], but both placeable values are themselves
    /// **message keys**, resolved in the same locale before interpolation.
    ///
    /// Exists because the two sub-messages cannot be composed in the
    /// template: writing `t_arg2(k, "a", t(k1), "b", t(k2))` there makes
    /// Askama hand `t_arg2` a `&&String`, which does not satisfy
    /// `Into<FluentValue<'static>>`. Resolving here keeps the call site
    /// readable and the composition in Rust, where it belongs.
    ///
    /// Fluent's own `{ message-ref }` syntax cannot do this: the reference
    /// has to be literal in the `.ftl`, and these two are chosen at runtime
    /// from the selected chart window and status class.
    pub fn t_arg2_msg(
        &self,
        key: &str,
        name1: &str,
        key1: &str,
        name2: &str,
        key2: &str,
    ) -> String {
        self.t_arg2(key, name1, self.t(key1), name2, self.t(key2))
    }
}

/// Lets the shared chrome partials resolve catalog keys against this request's
/// locale (#799).
///
/// A forwarding shim, not an adapter: the shared markup needs only the
/// *intersection* of the two products' `I18n` surfaces — a locale tag and a
/// message lookup — and both `helios_ui::I18n` and this type already spell
/// those exactly that way. Nothing is translated, renamed or defaulted here,
/// so the trait cannot become a place where the two implementations quietly
/// diverge. HTS-only methods (`t_or`, `t_arg`, `t_arg2`, `t_arg2_msg`)
/// deliberately stay off the trait: they are page-level vocabulary, and
/// widening the shared contract to fit them would force HFS to grow methods
/// its own templates never call.
impl helios_ui_chrome::ChromeLabels for I18n {
    fn lang(&self) -> String {
        I18n::lang(self)
    }

    fn t(&self, key: &str) -> String {
        I18n::t(self, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn default_locale_is_english() {
        let (locale, explicit) = negotiate(&Uri::from_static("/ui/hts"), &HeaderMap::new());
        assert_eq!(locale, &EN);
        assert!(!explicit);
    }

    #[test]
    fn accept_language_is_matched_by_rfc4647_lookup() {
        let map = headers(&[(header::ACCEPT_LANGUAGE, "de-DE, de;q=0.9, en;q=0.7")]);
        let (locale, _) = negotiate(&Uri::from_static("/ui/hts"), &map);
        assert_eq!(locale, &DE);
    }

    #[test]
    fn hts_lang_cookie_beats_accept_language() {
        let map = headers(&[
            (header::COOKIE, "session=abc; hts_lang=es"),
            (header::ACCEPT_LANGUAGE, "de"),
        ]);
        let (locale, explicit) = negotiate(&Uri::from_static("/ui/hts"), &map);
        assert_eq!(locale, &ES);
        assert!(!explicit);
    }

    #[test]
    fn query_override_beats_everything_and_is_explicit() {
        let map = headers(&[
            (header::COOKIE, "hts_lang=es"),
            (header::ACCEPT_LANGUAGE, "de"),
        ]);
        let (locale, explicit) = negotiate(&Uri::from_static("/ui/hts?lang=de"), &map);
        assert_eq!(locale, &DE);
        assert!(explicit);
    }

    /// `t_or` exists because keys composed from server-supplied values form
    /// an open set. `t` renders the *key* on a miss, which is right for the
    /// UI's own hardcoded keys and wrong for these: an OperationOutcome
    /// carrying an issue code the catalog has no sentence for used to print
    /// `hts-outcome-code-business-rule` at the reader.
    #[test]
    fn t_or_falls_back_to_the_value_not_the_key() {
        let i18n = I18n::new(RequestLocale::default());

        // A code the catalog does carry still gets its sentence.
        assert_eq!(
            i18n.t_or("hts-outcome-code-not-found", "not-found"),
            i18n.t("hts-outcome-code-not-found"),
        );
        assert_ne!(
            i18n.t_or("hts-outcome-code-not-found", "not-found"),
            "not-found",
            "a translated code must not degrade to its raw form",
        );

        // A code it does not carry surfaces as itself, never as the key.
        assert_eq!(
            i18n.t_or("hts-outcome-code-business-rule", "business-rule"),
            "business-rule",
        );
        assert_eq!(
            i18n.t("hts-outcome-code-business-rule"),
            "hts-outcome-code-business-rule",
            "plain `t` still echoes the key — that is why `t_or` exists",
        );
    }

    /// Guards against the mojibake regression fixed 2026-08-19: the shared
    /// Fluent catalogs at `locales/{en,es,de}/main.ftl` had been round-tripped
    /// through cp1252 at some point, so every non-ASCII character rendered as
    /// double-encoded UTF-8 in the browser (e.g. `InglÃ©s` instead of
    /// `Inglés`, `â€"` instead of `—`). If a future edit re-introduces the
    /// corruption — for example by opening a `.ftl` in an editor that
    /// silently converts the file — this test fails loudly on the exact
    /// keys the language switcher renders.
    #[test]
    fn spanish_language_switcher_labels_are_valid_utf8() {
        let i18n = I18n { locale: &ES };
        assert_eq!(i18n.t("language-en"), "Inglés");
        assert_eq!(i18n.t("language-es"), "Español");
        assert_eq!(i18n.t("language-de"), "Alemán");
    }

    #[test]
    fn locale_catalogs_contain_no_mojibake_markers() {
        // A hit on any of these byte sequences means the catalog was
        // written through cp1252 mojibake. Checked across all three
        // shipped locales via the Fluent loader so the assertion travels
        // with any future locale additions.
        let mojibake_pairs = ["Ã©", "Ã¡", "Ã±", "â€", "Ã¢", "Â "];
        for locale in [&EN, &ES, &DE] {
            let i18n = I18n { locale };
            for key in [
                "language-en",
                "language-es",
                "language-de",
                "home-lede",
                "hts-nav-home",
            ] {
                let rendered = i18n.t(key);
                for pair in &mojibake_pairs {
                    assert!(
                        !rendered.contains(pair),
                        "{}.ftl key `{}` contains mojibake sequence `{}` \
                         (rendered: {rendered:?}). See `locales/{}/main.ftl`.",
                        locale,
                        key,
                        pair,
                        locale,
                    );
                }
            }
        }
    }
}
