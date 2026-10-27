//! Case- and accent-insensitive text folding for string search.
//!
//! FHIR string search is case-insensitive **and accent-insensitive**. We fold
//! by lowercasing and stripping Unicode combining marks (NFD decomposition, then
//! dropping `Mn` characters), so e.g. `Müller`, `muller`, and `MÜLLER` all fold
//! to `muller`. The folded form is stored alongside the raw value in the search
//! index and compared against a folded query value.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Folds text for accent- and case-insensitive comparison.
///
/// Lowercases, then NFD-decomposes and removes combining marks (diacritics).
pub fn fold_text(input: &str) -> String {
    input
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case_and_accents() {
        assert_eq!(fold_text("Müller"), "muller");
        assert_eq!(fold_text("MÜLLER"), "muller");
        assert_eq!(fold_text("muller"), "muller");
        assert_eq!(fold_text("Café"), "cafe");
        assert_eq!(fold_text("naïve"), "naive");
        assert_eq!(fold_text("ÀÉÎÕÜ"), "aeiou");
    }

    #[test]
    fn leaves_plain_ascii_unchanged_except_case() {
        assert_eq!(fold_text("Smith"), "smith");
        assert_eq!(fold_text("smith"), "smith");
    }
}
