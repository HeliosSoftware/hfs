//! Locale negotiation and Fluent catalog lookup for the web UI.
//!
//! One locale is negotiated per request and carried in request extensions as
//! [`RequestLocale`]; templates resolve catalog keys against it through the
//! [`I18n`] helper. The policy is the one ratified in `docs/multi-language.md`:
//! explicit `?lang=` override (persisted in the `hfs_lang` cookie by the
//! language switcher) → cookie → `Accept-Language` (RFC 4647 Lookup) → `en`.
//!
//! Catalogs live in `locales/<locale>/main.ftl` at the workspace root and are
//! embedded at compile time — no runtime file or CDN dependency, matching the
//! asset stance of this crate. `en` is the source locale and the final
//! fallback: a key missing from a translation renders its English string,
//! never a blank or a crash.

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

/// Locales the UI can render, in switcher order. `en` first: it is the
/// source locale and the default.
static SUPPORTED: [&LanguageIdentifier; 3] = [&EN, &ES, &DE];

/// Name of the cookie the language switcher sets via `?lang=`.
const LANG_COOKIE: &str = "hfs_lang";

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
/// switcher), persist it in the `hfs_lang` cookie so it survives navigation.
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

/// `hfs_lang` cookie value, if any.
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
    // Stable sort: equal-quality ranges keep the client's order.
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
///
/// Templates hold keys, never English text: `{{ i18n.t("nav-dashboard") }}`.
/// Values are interpolated as Fluent placeables (escaped by Askama on
/// output), never spliced into markup.
#[derive(Clone, Copy)]
pub struct I18n {
    locale: &'static LanguageIdentifier,
}

impl I18n {
    pub fn new(locale: RequestLocale) -> Self {
        I18n { locale: locale.0 }
    }

    /// Test-only constructor from a raw tag, for rendering templates in a
    /// specific locale without building a request.
    #[cfg(test)]
    pub(crate) fn from_tag(tag: &str) -> Option<Self> {
        match_supported(tag).map(|locale| I18n { locale })
    }

    /// BCP 47 tag of the active locale — for `<html lang>` and the switcher.
    pub fn lang(&self) -> String {
        self.locale.to_string()
    }

    /// Look up a message. Missing keys fall back to `en` (the loader's
    /// fallback language); an unknown key renders the key itself rather
    /// than crashing or blanking the page.
    pub fn t(&self, key: &str) -> String {
        LOCALES
            .try_lookup(self.locale, key)
            .unwrap_or_else(|| key.to_owned())
    }

    /// Look up a message with one named placeable, e.g.
    /// `t_arg("status-last-checked", "timestamp", status.checked_at)`.
    pub fn t_arg(&self, key: &str, name: &str, value: impl Into<FluentValue<'static>>) -> String {
        let args: HashMap<Cow<'static, str>, FluentValue<'static>> =
            HashMap::from([(Cow::Owned(name.to_owned()), value.into())]);
        LOCALES
            .try_lookup_with_args(self.locale, key, &args)
            .unwrap_or_else(|| key.to_owned())
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
        let (locale, explicit) = negotiate(&Uri::from_static("/ui"), &HeaderMap::new());
        assert_eq!(locale, &EN);
        assert!(!explicit);
    }

    #[test]
    fn accept_language_is_matched_by_rfc4647_lookup() {
        let map = headers(&[(header::ACCEPT_LANGUAGE, "de-DE, de;q=0.9, en;q=0.7")]);
        let (locale, _) = negotiate(&Uri::from_static("/ui"), &map);
        assert_eq!(locale, &DE, "de-DE must truncate to supported de");
    }

    #[test]
    fn accept_language_honors_quality_order() {
        let map = headers(&[(header::ACCEPT_LANGUAGE, "fr;q=0.9, es;q=0.8, en;q=0.1")]);
        let (locale, _) = negotiate(&Uri::from_static("/ui"), &map);
        assert_eq!(locale, &ES, "unsupported fr must fall through to es");
    }

    #[test]
    fn cookie_beats_accept_language() {
        let map = headers(&[
            (header::COOKIE, "session=abc; hfs_lang=es"),
            (header::ACCEPT_LANGUAGE, "de"),
        ]);
        let (locale, explicit) = negotiate(&Uri::from_static("/ui"), &map);
        assert_eq!(locale, &ES);
        assert!(!explicit, "a cookie is a saved choice, not a new override");
    }

    #[test]
    fn query_override_beats_everything_and_is_explicit() {
        let map = headers(&[
            (header::COOKIE, "hfs_lang=es"),
            (header::ACCEPT_LANGUAGE, "de"),
        ]);
        let (locale, explicit) = negotiate(&Uri::from_static("/ui/status?lang=de"), &map);
        assert_eq!(locale, &DE);
        assert!(explicit);
    }

    #[test]
    fn unsupported_values_fall_through() {
        let map = headers(&[
            (header::COOKIE, "hfs_lang=klingon"),
            (header::ACCEPT_LANGUAGE, "pt-BR, xx"),
        ]);
        let (locale, explicit) = negotiate(&Uri::from_static("/ui?lang=fr"), &map);
        assert_eq!(locale, &EN, "nothing supported anywhere → en default");
        assert!(!explicit, "an unsupported ?lang= must not set a cookie");
    }

    #[test]
    fn catalog_lookup_translates_and_falls_back() {
        let es = I18n { locale: &ES };
        assert_eq!(es.t("nav-resources"), "Recursos");
        let de = I18n { locale: &DE };
        assert_eq!(de.t("nav-signout"), "Abmelden");
        // Unknown keys render the key, never a blank.
        assert_eq!(de.t("no-such-key"), "no-such-key");
    }

    #[test]
    fn placeables_interpolate_per_locale() {
        let en = I18n { locale: &EN };
        assert_eq!(en.t_arg("health-uptime", "duration", "3d"), "Uptime: 3d");
        let de = I18n { locale: &DE };
        assert_eq!(
            de.t_arg("health-uptime", "duration", "3d"),
            "Betriebszeit: 3d"
        );
    }

    #[test]
    fn plural_categories_select_per_cldr() {
        let en = I18n { locale: &EN };
        assert_eq!(en.t_arg("resource-count", "count", 1), "1 resource");
        assert_eq!(en.t_arg("resource-count", "count", 2), "2 resources");
        let de = I18n { locale: &DE };
        assert_eq!(de.t_arg("resource-count", "count", 1), "1 Ressource");
        assert_eq!(de.t_arg("resource-count", "count", 5), "5 Ressourcen");
    }

    /// `en` is the source of truth: every locale must define exactly the
    /// same message and term set (locales/README.md).
    #[test]
    fn catalogs_share_the_same_key_set() {
        use fluent_syntax::ast::Entry;
        use std::collections::BTreeSet;
        use std::path::Path;

        let keys = |locale: &str| -> BTreeSet<String> {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../locales")
                .join(locale)
                .join("main.ftl");
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let resource = fluent_syntax::parser::parse(source)
                .unwrap_or_else(|(_, errors)| panic!("{locale}/main.ftl: {errors:?}"));
            resource
                .body
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Message(m) => Some(m.id.name.clone()),
                    Entry::Term(t) => Some(format!("-{}", t.id.name)),
                    _ => None,
                })
                .collect()
        };

        let en = keys("en");
        assert!(!en.is_empty());
        for locale in ["es", "de"] {
            assert_eq!(
                en,
                keys(locale),
                "{locale}/main.ftl must define the same key set as en/main.ftl"
            );
        }
    }
}
