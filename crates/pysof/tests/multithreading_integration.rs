//! Integration test for multithreading functionality

use helios_sof::{RunOptions, ContentType};
use chrono::Utc;

#[test]
fn test_run_options_threading_support() {
    // Test that RunOptions can be constructed
    let options = RunOptions {
        since: None,
        limit: Some(100),
        page: Some(1),
        ..Default::default()
    };

    assert_eq!(options.limit, Some(100));
    assert_eq!(options.page, Some(1));
}

#[test]
fn test_content_type_parsing_for_multithreading() {
    // Test that ContentType parsing works for different output formats
    // that might be used in multithreaded scenarios
    let formats = ["json", "csv", "ndjson"];
    
    for format in &formats {
        let content_type = ContentType::from_string(format);
        assert!(content_type.is_ok(), "Failed to parse format: {}", format);
    }
}

#[test]
fn test_default_run_options_compatibility() {
    // Test that Default::default() works
    let options = RunOptions::default();
    
    assert!(options.since.is_none());
    assert!(options.limit.is_none());
    assert!(options.page.is_none());
}

#[test]
fn test_run_options_cloning() {
    // Test that RunOptions can be cloned (important for multithreading)
    let original = RunOptions {
        since: Some(Utc::now()),
        limit: Some(50),
        page: Some(2),
        ..Default::default()
    };
    
    let cloned = original.clone();
    
    assert_eq!(original.limit, cloned.limit);
    assert_eq!(original.page, cloned.page);
    assert_eq!(original.since, cloned.since);
}