//! Test RunOptions functionality

use helios_sof::RunOptions;
use chrono::{DateTime, Utc};

#[test]
fn test_run_options_basic() {
    // Test that we can construct RunOptions with basic fields
    let options = RunOptions {
        since: None,
        limit: Some(100),
        page: Some(1),
        ..Default::default()
    };

    assert_eq!(options.limit, Some(100));
    assert_eq!(options.page, Some(1));
    assert!(options.since.is_none());
}

#[test]
fn test_run_options_default() {
    // Test that default RunOptions has None for all fields
    let options = RunOptions::default();
    
    assert!(options.since.is_none());
    assert!(options.limit.is_none());
    assert!(options.page.is_none());
}

#[test]
fn test_run_options_partial_update() {
    // Test that we can update specific fields while keeping defaults
    let options = RunOptions {
        limit: Some(50),
        ..Default::default()
    };

    assert_eq!(options.limit, Some(50));
    assert!(options.since.is_none());
    assert!(options.page.is_none());
}

#[test]
fn test_run_options_with_since() {
    // Test that we can use since with other options
    let since_time = "2024-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
    
    let options = RunOptions {
        since: Some(since_time),
        ..Default::default()
    };

    assert_eq!(options.since, Some(since_time));
    assert!(options.limit.is_none());
    assert!(options.page.is_none());
}