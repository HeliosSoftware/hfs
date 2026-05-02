//! Retry logic with exponential backoff.

use std::time::Duration;

use crate::config::SubscriptionConfig;

/// Calculates the delay before the next retry attempt using exponential backoff.
///
/// delay = min(initial_delay * backoff_factor^attempt, max_delay)
pub fn calculate_delay(config: &SubscriptionConfig, attempt: u32) -> Duration {
    let delay_secs =
        config.retry_initial_delay.as_secs_f64() * config.retry_backoff_factor.powi(attempt as i32);

    let capped = delay_secs.min(config.retry_max_delay.as_secs_f64());

    Duration::from_secs_f64(capped)
}

/// Returns whether another retry should be attempted.
pub fn should_retry(config: &SubscriptionConfig, attempt: u32) -> bool {
    attempt < config.max_retries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SubscriptionConfig {
        SubscriptionConfig {
            max_retries: 5,
            retry_initial_delay: Duration::from_secs(1),
            retry_max_delay: Duration::from_secs(60),
            retry_backoff_factor: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn test_calculate_delay_exponential() {
        let config = test_config();
        assert_eq!(calculate_delay(&config, 0), Duration::from_secs(1)); // 1 * 2^0 = 1
        assert_eq!(calculate_delay(&config, 1), Duration::from_secs(2)); // 1 * 2^1 = 2
        assert_eq!(calculate_delay(&config, 2), Duration::from_secs(4)); // 1 * 2^2 = 4
        assert_eq!(calculate_delay(&config, 3), Duration::from_secs(8)); // 1 * 2^3 = 8
        assert_eq!(calculate_delay(&config, 4), Duration::from_secs(16)); // 1 * 2^4 = 16
    }

    #[test]
    fn test_calculate_delay_capped() {
        let config = test_config();
        // 1 * 2^10 = 1024, but max is 60.
        assert_eq!(calculate_delay(&config, 10), Duration::from_secs(60));
    }

    #[test]
    fn test_should_retry() {
        let config = test_config();
        assert!(should_retry(&config, 0));
        assert!(should_retry(&config, 4));
        assert!(!should_retry(&config, 5));
        assert!(!should_retry(&config, 10));
    }
}
