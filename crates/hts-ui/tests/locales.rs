//! Fluent catalog invariants for `locales/{en,es,de}/main.ftl`.
//!
//! Every user-visible string in the HTS UI is a catalog key (#551 D5), and
//! `I18n::t` renders an unknown key *as the key itself* rather than crashing.
//! That failure mode is invisible in a passing render test, so the catalogs
//! are checked structurally here instead:
//!
//! - all three locales define exactly the same key set, so switching language
//!   can never drop a label;
//! - every `.ftl` still parses, so a bad edit fails the ring rather than
//!   silently truncating the catalog at the malformed entry.
//!
//! `en` is the source locale and the fallback, so a key present only in a
//! translation is just as much a bug as one missing from it — it means the
//! English string it was translated from has been removed or renamed.

use fluent_syntax::ast;
use fluent_syntax::parser;
use std::collections::BTreeSet;

const LOCALES: [&str; 3] = ["en", "es", "de"];

/// Message and term identifiers defined by one catalog.
fn key_set(locale: &str) -> BTreeSet<String> {
    let path = format!("../../locales/{locale}/main.ftl");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path} must be readable: {e}"));
    // A `.ftl` with syntax errors still yields the entries it managed to
    // parse; treat any error as fatal so a malformed catalog cannot quietly
    // pass as a smaller one.
    let resource = parser::parse(source.as_str()).unwrap_or_else(|(_, errors)| {
        panic!("{path} failed to parse: {errors:?}");
    });
    resource
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Message(m) => Some(m.id.name.to_string()),
            ast::Entry::Term(t) => Some(format!("-{}", t.id.name)),
            _ => None,
        })
        .collect()
}

#[test]
fn every_locale_defines_the_same_key_set() {
    let en = key_set("en");
    assert!(!en.is_empty(), "the English catalog must not be empty");
    for locale in LOCALES.iter().filter(|l| **l != "en") {
        let other = key_set(locale);
        let missing: Vec<&String> = en.difference(&other).collect();
        let extra: Vec<&String> = other.difference(&en).collect();
        assert!(
            missing.is_empty(),
            "locales/{locale}/main.ftl is missing {} key(s) present in en: {missing:?}",
            missing.len(),
        );
        assert!(
            extra.is_empty(),
            "locales/{locale}/main.ftl defines {} key(s) that no longer exist in en \
             (renamed or pruned upstream?): {extra:?}",
            extra.len(),
        );
    }
}

#[test]
fn the_home_chart_keys_exist_in_every_locale() {
    // Named explicitly because 153 dead keys were pruned recently: a key that
    // exists nowhere renders as its own name in the UI, and the parity test
    // above would happily accept it being absent from all three.
    const CHART_KEYS: [&str; 20] = [
        "hts-home-chart-title",
        "hts-home-chart-window",
        "hts-home-chart-series",
        "hts-home-chart-window-15m",
        "hts-home-chart-window-1h",
        "hts-home-chart-window-6h",
        "hts-home-chart-series-all",
        "hts-home-chart-series-2xx",
        "hts-home-chart-series-4xx",
        "hts-home-chart-series-5xx",
        "hts-home-chart-hint",
        "hts-home-chart-empty-unreachable",
        "hts-home-chart-empty-none",
        "hts-home-chart-empty-first",
        "hts-home-chart-empty-window",
        "hts-home-chart-axis-now",
        "hts-home-chart-axis-minutes",
        "hts-home-chart-axis-hours",
        // Pre-existing tile keys the chart card reuses.
        "hts-home-tile-metrics-hint",
        "hts-home-subtitle",
    ];
    for locale in LOCALES {
        let keys = key_set(locale);
        for key in CHART_KEYS {
            assert!(
                keys.contains(key),
                "`{key}` is missing from locales/{locale}/main.ftl",
            );
        }
    }
}
