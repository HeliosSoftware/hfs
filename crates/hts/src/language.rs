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
/// Lower rank is a better match; `None` means no match. For each truncation
/// of the requested tag (`de-DE-1996` → `de-DE` → `de`), in order:
///
/// * the stored tag equals the candidate (case-insensitive) — rank `2*i`;
/// * the stored tag extends the candidate with a `-` subtag (so `de`
///   accepts `de-CH` but not `den`) — rank `2*i + 1`.
///
/// Examples (requested → stored): `de`→`de` is 0, `de`→`de-CH` is 1,
/// `de-DE`→`de` is 2, `de-DE`→`de-CH` is 3, `de`→`den` is `None`.
pub(crate) fn lang_match_rank(requested: &str, stored: &str) -> Option<u32> {
    let stored = stored.to_ascii_lowercase();
    let mut candidate = requested.trim().to_ascii_lowercase();
    let mut i = 0u32;
    loop {
        if stored == candidate {
            return Some(2 * i);
        }
        if stored.len() > candidate.len()
            && stored.starts_with(candidate.as_str())
            && stored.as_bytes()[candidate.len()] == b'-'
        {
            return Some(2 * i + 1);
        }
        match candidate.rfind('-') {
            Some(pos) => {
                candidate.truncate(pos);
                i += 1;
            }
            None => return None,
        }
    }
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
    fn stored_dialect_satisfies_bare_request() {
        assert_eq!(lang_match_rank("de", "de-CH"), Some(1));
        assert_eq!(lang_match_rank("fr", "fr-FR"), Some(1));
        // No subtag boundary — must not match.
        assert_eq!(lang_match_rank("de", "den"), None);
    }

    #[test]
    fn requested_dialect_truncates_to_bare_stored() {
        assert_eq!(lang_match_rank("de-DE", "de"), Some(2));
        assert_eq!(lang_match_rank("de-DE", "de-CH"), Some(3));
        assert_eq!(lang_match_rank("zh-Hans-CN", "zh"), Some(4));
    }

    #[test]
    fn unrelated_languages_do_not_match() {
        assert_eq!(lang_match_rank("de", "en"), None);
        assert_eq!(lang_match_rank("de-DE", "en-US"), None);
        assert!(!lang_matches("sv-SE", "da"));
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
