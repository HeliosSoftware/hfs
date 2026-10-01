//! Locale number formatting shared by both UIs.
//!
//! A figure reads the way the page's own locale writes it: `70,048` and `1.5`
//! in English, `70.048` and `1,5` in German and Spanish. These are the CLDR
//! rules the browser applies through `Number.prototype.toLocaleString`, which
//! is what the client-side half (`crates/ui/assets/number.js`) calls — so a
//! number rendered on the server and one written by a script on the same
//! page agree, including Spanish leaving four-digit figures ungrouped
//! (`1234`, but `12.345`: CLDR's minimum grouping digits is 2 for `es`).
//!
//! Two ways in:
//!
//! * [`integer`] / [`decimal`] for a template or handler that prints a
//!   number itself;
//! * [`fluent_formatter`], installed on each Fluent bundle, so every numeric
//!   placeable a message interpolates is formatted the same way while the raw
//!   number still selects the plural form.
//!
//! Only displayed text is formatted. Identifiers — HTTP statuses, ports,
//! years, version ids — and wire values must reach the page as strings, not
//! numbers, so neither path touches them.

use fluent_bundle::FluentValue;
use intl_memoizer::Memoizable;
use intl_memoizer::concurrent::IntlLangMemoizer;
use unic_langid::LanguageIdentifier;

/// The separators one locale uses. Unknown languages fall back to English,
/// the UI's source locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbols {
    group: char,
    decimal: char,
    /// CLDR `minimumGroupingDigits`: an integer part needs at least
    /// `3 + min_grouping` digits before it is grouped at all.
    min_grouping: usize,
}

impl Symbols {
    /// Separators for a BCP 47 tag (`"de"`, `"es-MX"`, `"en"`).
    pub fn for_lang(lang: &str) -> Self {
        let language = lang.split(['-', '_']).next().unwrap_or("");
        match language.to_ascii_lowercase().as_str() {
            "de" => Symbols {
                group: '.',
                decimal: ',',
                min_grouping: 1,
            },
            "es" => Symbols {
                group: '.',
                decimal: ',',
                min_grouping: 2,
            },
            _ => Symbols {
                group: ',',
                decimal: '.',
                min_grouping: 1,
            },
        }
    }

    /// Lays out an already-rendered plain number (`-1234.5`: ASCII digits,
    /// an optional leading `-`, an optional `.` fraction) with this locale's
    /// separators. Anything else comes back unchanged.
    fn localize(&self, plain: &str, grouping: bool) -> String {
        let (sign, unsigned) = match plain.strip_prefix('-') {
            Some(rest) => ("-", rest),
            None => ("", plain),
        };
        let (int, frac) = match unsigned.split_once('.') {
            Some((int, frac)) => (int, Some(frac)),
            None => (unsigned, None),
        };
        let is_digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        if !is_digits(int) || frac.is_some_and(|f| !is_digits(f)) {
            return plain.to_owned();
        }

        let mut out = String::with_capacity(plain.len() + plain.len() / 3);
        out.push_str(sign);
        if grouping && int.len() >= 3 + self.min_grouping {
            for (i, digit) in int.chars().enumerate() {
                if i > 0 && (int.len() - i).is_multiple_of(3) {
                    out.push(self.group);
                }
                out.push(digit);
            }
        } else {
            out.push_str(int);
        }
        if let Some(frac) = frac {
            out.push(self.decimal);
            out.push_str(frac);
        }
        out
    }
}

/// An integer with the locale's grouping: `integer(70048, "de") == "70.048"`.
pub fn integer(n: impl Into<i128>, lang: &str) -> String {
    Symbols::for_lang(lang).localize(&n.into().to_string(), true)
}

/// A decimal rounded to exactly `fraction_digits` places, with the locale's
/// grouping and decimal separator: `decimal(1234.56, 1, "de") == "1.234,6"`.
/// A non-finite value is rendered as Rust prints it.
pub fn decimal(x: f64, fraction_digits: usize, lang: &str) -> String {
    Symbols::for_lang(lang).localize(&format!("{x:.fraction_digits$}"), true)
}

/// An integer a host's `I18n::num` can format: every primitive integer type, by
/// value or by reference (Askama hands loop and `if let` bindings over as
/// references).
pub trait UiNumber {
    fn ui_number(&self) -> i128;
}

macro_rules! ui_number {
    ($($t:ty),*) => {$(
        impl UiNumber for $t {
            fn ui_number(&self) -> i128 {
                *self as i128
            }
        }
    )*};
}
ui_number!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl<T: UiNumber + ?Sized> UiNumber for &T {
    fn ui_number(&self) -> i128 {
        (**self).ui_number()
    }
}

/// A decimal a host's `I18n::dec` can format: `f64`/`f32`, by value or by
/// any depth of reference, for the same Askama reason as [`UiNumber`].
pub trait UiDecimal {
    fn ui_decimal(&self) -> f64;
}

impl UiDecimal for f64 {
    fn ui_decimal(&self) -> f64 {
        *self
    }
}

impl UiDecimal for f32 {
    fn ui_decimal(&self) -> f64 {
        f64::from(*self)
    }
}

impl<T: UiDecimal + ?Sized> UiDecimal for &T {
    fn ui_decimal(&self) -> f64 {
        (**self).ui_decimal()
    }
}

/// The per-locale state [`fluent_formatter`] keeps in a bundle's memoizer —
/// the memoizer is how a plain `fn` formatter learns the bundle's language.
struct FluentSymbols(Symbols);

impl Memoizable for FluentSymbols {
    type Args = ();
    type Error = ();

    fn construct(lang: LanguageIdentifier, _args: ()) -> Result<Self, ()> {
        Ok(FluentSymbols(Symbols::for_lang(lang.language.as_str())))
    }
}

/// A Fluent number formatter (`FluentBundle::set_formatter`): every numeric
/// placeable comes out in the bundle's locale. Plural selection is untouched —
/// Fluent selects on the number itself, this only changes how it is printed.
///
/// Honors the message's own `NUMBER()` options that matter here:
/// `useGrouping: "false"` and `minimumFractionDigits`/`maximumFractionDigits`.
pub fn fluent_formatter(value: &FluentValue, intls: &IntlLangMemoizer) -> Option<String> {
    let FluentValue::Number(number) = value else {
        return None;
    };
    let options = &number.options;
    let plain = match options.maximum_fraction_digits {
        Some(max) => {
            let min = options.minimum_fraction_digits.unwrap_or(0).min(max);
            let rounded = format!("{:.max$}", number.value);
            trim_fraction(&rounded, min)
        }
        None => number.as_string().into_owned(),
    };
    intls
        .with_try_get::<FluentSymbols, _, _>((), |symbols| {
            symbols.0.localize(&plain, options.use_grouping)
        })
        .ok()
}

/// Drops trailing fraction zeros down to `min` places (`"1.50"` with `min` 0
/// → `"1.5"`, `"2.00"` → `"2"`).
fn trim_fraction(plain: &str, min: usize) -> String {
    let Some((int, frac)) = plain.split_once('.') else {
        return plain.to_owned();
    };
    let keep = frac.trim_end_matches('0').len().max(min);
    if keep == 0 {
        int.to_owned()
    } else {
        format!("{int}.{}", &frac[..keep.min(frac.len())])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent_bundle::concurrent::FluentBundle;
    use fluent_bundle::{FluentArgs, FluentResource};

    #[test]
    fn groups_integers_per_locale() {
        assert_eq!(integer(70_048u64, "en"), "70,048");
        assert_eq!(integer(70_048u64, "de"), "70.048");
        assert_eq!(integer(70_048u64, "es"), "70.048");
        assert_eq!(integer(1_234_567u64, "en-US"), "1,234,567");
        assert_eq!(integer(-1_234_567i64, "de"), "-1.234.567");
        assert_eq!(integer(999u32, "en"), "999");
        assert_eq!(integer(0u8, "de"), "0");
    }

    #[test]
    fn spanish_leaves_four_digit_figures_ungrouped() {
        // CLDR minimumGroupingDigits = 2 for es — what the browser does too.
        assert_eq!(integer(1_234u64, "es"), "1234");
        assert_eq!(integer(12_345u64, "es"), "12.345");
        assert_eq!(integer(1_234u64, "de"), "1.234");
        assert_eq!(integer(1_234u64, "en"), "1,234");
    }

    #[test]
    fn unknown_languages_fall_back_to_english() {
        assert_eq!(integer(70_048u64, "fr"), "70,048");
        assert_eq!(integer(70_048u64, ""), "70,048");
    }

    #[test]
    fn decimals_use_the_locale_separator() {
        assert_eq!(decimal(1.5, 1, "en"), "1.5");
        assert_eq!(decimal(1.5, 1, "de"), "1,5");
        assert_eq!(decimal(1_234.56, 1, "de"), "1.234,6");
        assert_eq!(decimal(1_234.56, 1, "es"), "1234,6");
        assert_eq!(decimal(12.0, 0, "en"), "12");
        assert_eq!(decimal(f64::NAN, 1, "en"), "NaN");
    }

    fn render(lang: &str, source: &str, n: impl Into<FluentValue<'static>>) -> String {
        let langid: LanguageIdentifier = lang.parse().unwrap();
        let mut bundle = FluentBundle::new_concurrent(vec![langid]);
        bundle.set_use_isolating(false);
        bundle.set_formatter(Some(fluent_formatter));
        bundle.add_builtins().unwrap();
        bundle
            .add_resource(FluentResource::try_new(source.to_owned()).unwrap())
            .unwrap();
        let mut args = FluentArgs::new();
        args.set("n", n);
        let message = bundle.get_message("m").unwrap();
        let mut errors = vec![];
        bundle
            .format_pattern(message.value().unwrap(), Some(&args), &mut errors)
            .into_owned()
    }

    #[test]
    fn fluent_placeables_are_grouped_and_still_select_the_plural() {
        let source = "m = { $n -> \n    [one] { $n } result\n   *[other] { $n } results\n}\n";
        assert_eq!(render("en", source, 70_048u64), "70,048 results");
        assert_eq!(render("de", source, 70_048u64), "70.048 results");
        assert_eq!(render("es", source, 1_234u64), "1234 results");
        assert_eq!(render("en", source, 1u64), "1 result");
    }

    #[test]
    fn fluent_strings_are_left_alone() {
        // Identifiers travel as strings and must never be grouped.
        assert_eq!(render("en", "m = HTTP { $n }\n", "40400"), "HTTP 40400");
    }

    #[test]
    fn fluent_number_options_are_honored() {
        assert_eq!(
            render(
                "en",
                "m = { NUMBER($n, useGrouping: \"false\") }\n",
                2026u64
            ),
            "2026"
        );
        assert_eq!(
            render(
                "de",
                "m = { NUMBER($n, maximumFractionDigits: 1) }\n",
                1_234.56
            ),
            "1.234,6"
        );
        assert_eq!(
            render("en", "m = { NUMBER($n, maximumFractionDigits: 2) }\n", 1.5),
            "1.5"
        );
        assert_eq!(render("de", "m = { $n }\n", 0.25), "0,25");
    }
}
