//! Path/method exclusion rules for skipping audit on certain requests.

/// A single exclusion rule: skip audit when the request matches.
#[derive(Debug, Clone)]
pub struct ExclusionRule {
    /// URL path to match (e.g. `"/health"`). Trailing slashes are ignored.
    pub path: String,
    /// HTTP method to match (e.g. `"GET"`). `None` matches all methods.
    pub method: Option<String>,
}

/// Collection of exclusion rules applied before recording an audit event.
#[derive(Debug, Clone)]
pub struct ExclusionFilter {
    rules: Vec<ExclusionRule>,
}

impl ExclusionFilter {
    /// Create a filter from a list of rules.
    pub fn new(rules: Vec<ExclusionRule>) -> Self {
        Self { rules }
    }

    /// Default exclusions for paths that should never generate audit events.
    pub fn default_exclusions() -> Self {
        Self {
            rules: vec![
                ExclusionRule {
                    path: "/health".to_string(),
                    method: None,
                },
                ExclusionRule {
                    path: "/metadata".to_string(),
                    method: None,
                },
                ExclusionRule {
                    path: "/.well-known/smart-configuration".to_string(),
                    method: None,
                },
                ExclusionRule {
                    path: "/$versions".to_string(),
                    method: None,
                },
            ],
        }
    }

    /// Returns `true` if the given path/method combination is excluded from audit.
    pub fn is_excluded(&self, path: &str, method: &str) -> bool {
        let normalized = path.trim_end_matches('/');
        self.rules.iter().any(|rule| {
            let path_match = normalized == rule.path.trim_end_matches('/')
                || normalized.starts_with(&format!("{}?", rule.path.trim_end_matches('/')));
            let method_match = rule
                .method
                .as_ref()
                .is_none_or(|m| m.eq_ignore_ascii_case(method));
            path_match && method_match
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_exclusions_health() {
        let filter = ExclusionFilter::default_exclusions();
        assert!(filter.is_excluded("/health", "GET"));
        assert!(filter.is_excluded("/health", "HEAD"));
    }

    #[test]
    fn test_default_exclusions_metadata() {
        let filter = ExclusionFilter::default_exclusions();
        assert!(filter.is_excluded("/metadata", "GET"));
    }

    #[test]
    fn test_default_exclusions_smart_config() {
        let filter = ExclusionFilter::default_exclusions();
        assert!(filter.is_excluded("/.well-known/smart-configuration", "GET"));
    }

    #[test]
    fn test_non_excluded_path() {
        let filter = ExclusionFilter::default_exclusions();
        assert!(!filter.is_excluded("/Patient", "GET"));
        assert!(!filter.is_excluded("/Patient/123", "PUT"));
    }

    #[test]
    fn test_trailing_slash_ignored() {
        let filter = ExclusionFilter::default_exclusions();
        assert!(filter.is_excluded("/health/", "GET"));
    }

    #[test]
    fn test_method_specific_exclusion() {
        let filter = ExclusionFilter::new(vec![ExclusionRule {
            path: "/some-path".to_string(),
            method: Some("GET".to_string()),
        }]);
        assert!(filter.is_excluded("/some-path", "GET"));
        assert!(!filter.is_excluded("/some-path", "POST"));
    }

    #[test]
    fn test_empty_filter_excludes_nothing() {
        let filter = ExclusionFilter::new(vec![]);
        assert!(!filter.is_excluded("/health", "GET"));
        assert!(!filter.is_excluded("/Patient", "POST"));
    }
}
