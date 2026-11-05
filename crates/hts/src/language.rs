//! BCP-47 / RFC 4647 language-tag matching for designation selection.
//!
//! Designation languages come from heterogeneous sources — SNOMED RF2 ships
//! bare primary tags (`de`, `da`), LOINC linguistic variants ship
//! region-qualified tags (`de-DE`, `fr-FR`) — while clients send whatever
//! their locale produces (browsers typically `de-DE` via `Accept-Language`).
//! Exact string equality therefore fails in both directions. The helpers
//! here implement RFC 4647 §3.4 *Lookup* with progressive truncation of the
//! requested tag, plus the extends-with-a-subtag rule already used by
//! `$expand` (requested `de` accepts stored `de-CH`).

/// Rank a stored designation language tag against a requested tag.
///
/// Lower rank is a better match; `None` means no match. The tiers, from best
/// to worst, encode the preference order "`es-ES`, then `esES`, then `es` or
/// `es*`" (and likewise for every language):
///
/// * **0 — exact:** the stored tag equals the requested tag, region and all
///   (`es-ES` → `es-ES`, case-insensitive).
/// * **1 — separator-insensitive exact:** the stored tag equals the requested
///   tag with separators removed (`es-ES` → `esES`).
/// * **2 — same primary language:** the stored tag is the bare primary subtag
///   of the request (`es-ES` → `es`) *or* any regional sibling/extension under
///   that primary (`es-ES` → `es-AR`, `es-MX`; `es` → `es-MX`). These all share
///   one tier, so a bare tag and its regional siblings are equal-preference —
///   "`es` or `es*`, whichever appears first". A more specific tier-0/1 match,
///   when one exists among the candidates, outranks and therefore excludes
///   them.
///
/// Examples (requested → stored): `es-ES`→`es-ES` is 0, `es-ES`→`esES` is 1,
/// `es-ES`→`es` is 2, `es-ES`→`es-MX` is 2, `de`→`de-CH` is 2, `de`→`den` is
/// `None`.
pub(crate) fn lang_match_rank(requested: &str, stored: &str) -> Option<u32> {
    let r = requested.trim().to_ascii_lowercase();
    let s = stored.trim().to_ascii_lowercase();

    // Tier 0: exact match (identical region included).
    if s == r {
        return Some(0);
    }
    // Tier 1: separator-insensitive exact match (es-ES vs esES). Only meaningful
    // when the request carries a separator; otherwise this collapses into tier 0.
    if r.contains('-') && s == r.replace('-', "") {
        return Some(1);
    }
    // Tier 2: same primary language subtag — either the bare primary itself or
    // any regional sibling/extension under it. The `de-DE` → bare `de` case
    // (SNOMED RF2 ships bare language codes) lands here too.
    let primary = r.split('-').next().unwrap_or(r.as_str());
    if !primary.is_empty()
        && (s == primary
            || (s.len() > primary.len()
                && s.starts_with(primary)
                && s.as_bytes()[primary.len()] == b'-'))
    {
        return Some(2);
    }
    None
}

/// `true` when the stored tag satisfies the requested tag under
/// [`lang_match_rank`].
pub(crate) fn lang_matches(requested: &str, stored: &str) -> bool {
    lang_match_rank(requested, stored).is_some()
}

/// Index of the best-matching language among `langs` for `requested`, or
/// `None` when nothing matches. Ties keep the earliest item, so callers that
/// order designations preferred-first retain that preference.
pub(crate) fn best_lang_match_index<'a>(
    requested: &str,
    langs: impl IntoIterator<Item = Option<&'a str>>,
) -> Option<usize> {
    let mut best: Option<(u32, usize)> = None;
    for (idx, lang) in langs.into_iter().enumerate() {
        if let Some(rank) = lang.and_then(|l| lang_match_rank(requested, l)) {
            if best.is_none_or(|(r, _)| rank < r) {
                best = Some((rank, idx));
            }
        }
    }
    best.map(|(_, idx)| idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_is_best() {
        assert_eq!(lang_match_rank("de", "de"), Some(0));
        assert_eq!(lang_match_rank("de-DE", "de-DE"), Some(0));
        assert_eq!(lang_match_rank("en-US", "en-us"), Some(0));
    }

    #[test]
    fn separator_insensitive_exact_is_tier_one() {
        // es-ES, then esES — the hyphen-less form ranks just below exact.
        assert_eq!(lang_match_rank("es-ES", "esES"), Some(1));
        assert_eq!(lang_match_rank("zh-Hans-CN", "zhHansCN"), Some(1));
        // For a bare request there is no separate hyphen-less form.
        assert_eq!(lang_match_rank("de", "de"), Some(0));
    }

    #[test]
    fn bare_primary_and_regional_siblings_share_tier_two() {
        // es-ES → es, and es-ES → es-AR / es-MX are all tier 2 ("es or es*").
        assert_eq!(lang_match_rank("es-ES", "es"), Some(2));
        assert_eq!(lang_match_rank("es-ES", "es-AR"), Some(2));
        assert_eq!(lang_match_rank("es-ES", "es-MX"), Some(2));
        // Bare request extends to any regional variant.
        assert_eq!(lang_match_rank("de", "de-CH"), Some(2));
        assert_eq!(lang_match_rank("fr", "fr-FR"), Some(2));
        // SNOMED RF2 ships bare codes: a de-DE request must accept stored `de`.
        assert_eq!(lang_match_rank("de-DE", "de"), Some(2));
        assert_eq!(lang_match_rank("zh-Hans-CN", "zh"), Some(2));
        // No subtag boundary — must not match.
        assert_eq!(lang_match_rank("de", "den"), None);
    }

    #[test]
    fn unrelated_languages_do_not_match() {
        assert_eq!(lang_match_rank("de", "en"), None);
        assert_eq!(lang_match_rank("de-DE", "en-US"), None);
        assert!(!lang_matches("sv-SE", "da"));
        // A region-specific request must not match a *different* region when an
        // unrelated primary is involved.
        assert_eq!(lang_match_rank("es-ES", "en-US"), None);
    }

    #[test]
    fn best_index_prefers_rank_then_order() {
        // Exact beats dialect-extension regardless of order.
        let langs = [Some("de-CH"), Some("de")];
        assert_eq!(best_lang_match_index("de", langs), Some(1));
        // Equal ranks keep the earliest (preferred-first ordering).
        let langs = [Some("de"), Some("de")];
        assert_eq!(best_lang_match_index("de", langs), Some(0));
        let langs = [Some("en"), None, Some("fr")];
        assert_eq!(best_lang_match_index("fr-FR", langs), Some(2));
        assert_eq!(best_lang_match_index("ja", langs), None);
    }
}
