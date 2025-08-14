#[cfg(feature = "R6")]
#[test]
fn test_real_json_with_string_integers() {
    use helios_fhir::r6::Bundle;
    use serde_json;

    // Simplified version of the problematic JSON with string integers
    let json_content = r#"{
  "resourceType": "Bundle",
  "id": "test-bundle",
  "type": "subscription-notification",
  "entry": [
    {
      "fullUrl": "urn:uuid:test-subscription-status",
      "resource": {
        "resourceType": "SubscriptionStatus",
        "id": "test-subscription-status",
        "status": "active",
        "type": "event-notification",
        "eventsSinceSubscriptionStart": "2",
        "notificationEvent": [
          {
            "eventNumber": "2",
            "focus": {
              "reference": "http://example.org/FHIR/R5/Encounter/2"
            }
          }
        ],
        "subscription": {
          "reference": "http://example.org/FHIR/R5/Subscription/123"
        },
        "topic": "http://example.org/FHIR/R5/SubscriptionTopic/admission"
      }
    }
  ]
}"#;

    // This should now succeed with our fix
    let result: Result<Bundle, _> = serde_json::from_str(json_content);
    assert!(
        result.is_ok(),
        "Failed to deserialize Bundle with string integers: {:?}",
        result.err()
    );

    let bundle = result.unwrap();
    assert_eq!(
        bundle.id.as_ref().and_then(|e| e.value.as_ref()),
        Some(&"test-bundle".to_string())
    );

    // Check that the integer fields were properly parsed
    if let Some(entries) = &bundle.entry {
        if let Some(entry) = entries.first() {
            if let Some(resource) = &entry.resource {
                match &**resource {
                    helios_fhir::r6::Resource::SubscriptionStatus(sub_status) => {
                        // Check events_since_subscription_start was parsed from string "2"
                        if let Some(events_count) = &sub_status.events_since_subscription_start {
                            if let Some(value) = &events_count.value {
                                assert_eq!(
                                    *value, 2i64,
                                    "eventsSinceSubscriptionStart should be 2"
                                );
                            }
                        }

                        // Check notification event eventNumber was parsed from string "2"
                        if let Some(notification_events) = &sub_status.notification_event {
                            if let Some(event) = notification_events.first() {
                                if let Some(event_num) = &event.event_number.value {
                                    assert_eq!(*event_num, 2i64, "eventNumber should be 2");
                                }
                            }
                        }
                    }
                    _ => panic!("Expected SubscriptionStatus resource"),
                }
            }
        }
    }
}
