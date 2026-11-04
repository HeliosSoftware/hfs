//! Opt-in refusal of an oversized transaction Bundle, before it is admitted
//! (#2351; `HFS_MONGODB_TRANSACTION_BUNDLE_MAX_PAYLOAD_BYTES`).
//!
//! A Bundle too large for the WiredTiger cache is otherwise found out only when
//! MongoDB rolls the transaction back, often after it has written most of its
//! entries while holding the whole admission room (#1976). With
//! `MongoBackendConfig::transaction_bundle_max_payload_bytes` set, the Bundle's
//! payload is added up first, as the very first step of
//! `process_transaction_with_patch_validator`: before the admission, before any
//! session and before any command is sent. Above the limit it ends at once as
//! [`TransactionError::TooLargeForCache`] with
//! [`TooLargeForCacheCause::PayloadOverLimit`]: nothing ran and nothing is
//! rolled back.
//!
//! # What is counted
//!
//! The BSON size of the resource of every POST and PUT entry, as the client sent
//! it (before reference pinning), once per entry. This is the size of the
//! document each of them would write to `resources`; the copy a create or update
//! also writes to `resource_history` is not counted again, so the knob stays
//! close to the unit of the sizing rule in the README (resource bytes; BSON is
//! larger than JSON, see "Computing the size"), and the factor of about 2 is a
//! constant the operator's value absorbs.
//!
//! - DELETE and GET carry no payload and count 0.
//! - PATCH is not counted: the document it writes is the patched target, known
//!   only after reading it inside the transaction, which is exactly what this
//!   refusal avoids. The patch body measures the change, not the write.
//! - A POST with `ifNoneExist`, or a conditional PUT, counts even when it would
//!   write nothing, so the sum is an upper bound.
//! - An entry whose resource is missing, or is not an object, counts 0.
//! - The `id` the server assigns to a create (when the Bundle's references are
//!   pinned, after this count) is not counted, so the stored document is
//!   slightly larger than its counted size.
//!
//! The Bundle is refused only when the sum is strictly greater than the limit.
//!
//! # Estimator not chosen
//!
//! The other estimator in the issue is 2 x payload (BSON) plus about 1.45 KB per
//! created document (#1976 decision record, section 2.3). It was not chosen:
//! the knob has to be settable straight from the README rule, which is in
//! resource bytes; the per-document constant depends on the index set, on search
//! offload and deferral (#1975) and on the server version, so baking it into
//! code would drift silently; and which model predicts the ceiling better was
//! not measured. The cost is that a bytes-only limit can misjudge Bundles of
//! many small resources, so an operator should leave headroom. A Bundle that
//! still overflows the cache ends on the existing 388 or lone-eviction paths.
//!
//! # Computing the size
//!
//! The size is computed arithmetically over the JSON value, mirroring how
//! `super::storage::value_to_document` (`bson::to_bson`) encodes it, so no
//! document is built and a large Bundle is counted in milliseconds. A unit test
//! pins the walker to the driver's own encoding.
//!
//! BSON is larger than compact JSON, and for number-heavy resources much
//! larger: the workspace builds `serde_json` with `arbitrary_precision`
//! (`helios-fhir` and `helios-serde` enable it through
//! `rust_decimal/serde-with-arbitrary-precision`), under which a number
//! serializes as a one-field struct and `bson::to_bson` stores it as a
//! sub-document, not as an Int32, Int64 or Double. See [`number_len`].

use std::fmt::{self, Write as _};

use serde_json::{Map, Number, Value};

use crate::core::{BundleEntry, BundleMethod};
use crate::error::{TooLargeForCacheCause, TransactionError};

/// BSON bytes of the resources a transaction Bundle would create or update:
/// the sum over its POST and PUT entries (see the module docs).
pub(super) fn bundle_payload_bytes(entries: &[BundleEntry]) -> u64 {
    entries
        .iter()
        .filter(|entry| matches!(entry.method, BundleMethod::Post | BundleMethod::Put))
        .filter_map(|entry| entry.resource.as_ref()?.as_object())
        .map(bson_document_len)
        .sum()
}

/// Encoded size of a BSON document: a 4-byte length, its elements and a
/// terminating NUL.
fn bson_document_len(map: &Map<String, Value>) -> u64 {
    5 + map
        .iter()
        .map(|(key, value)| bson_element_len(key.len(), value))
        .sum::<u64>()
}

/// Encoded size of one element: a type byte, the key and its NUL, the value.
fn bson_element_len(key_len: usize, value: &Value) -> u64 {
    2 + key_len as u64 + bson_value_len(value)
}

/// Encoded size of a value as `bson::to_bson` maps a `serde_json::Value`.
///
/// A string is a 4-byte length, its bytes and a NUL. An array is a document
/// whose keys are its decimal indices. A number is not a BSON number, see
/// [`number_len`].
fn bson_value_len(value: &Value) -> u64 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(number) => number_len(number),
        Value::String(text) => 5 + text.len() as u64,
        Value::Array(items) => {
            5 + items
                .iter()
                .enumerate()
                .map(|(index, item)| bson_element_len(decimal_digits(index), item))
                .sum::<u64>()
        }
        Value::Object(map) => bson_document_len(map),
    }
}

/// The key `serde_json` (with `arbitrary_precision`) gives the one field of a
/// serialized number.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// Encoded size of a JSON number: with `arbitrary_precision` on,
/// `serde_json::Number` serializes as a struct with one field,
/// [`NUMBER_TOKEN`], holding the number's text, so `bson::to_bson` makes a
/// sub-document `{"$serde_json::private::Number": "<text>"}`: a 5-byte document
/// around one string element, 40 bytes plus the text. The text is what
/// `Number`'s `Display` prints (the literal as parsed, or the shortest form of a
/// float built in code).
///
/// If the feature were ever switched off, a number would be an Int32, an Int64
/// or a Double and this would overcount: the first unit test below pins the
/// walker to the driver's own encoding and fails then.
fn number_len(number: &Number) -> u64 {
    let mut text = TextLen(0);
    // Writing to the counter cannot fail.
    let _ = write!(text, "{number}");
    5 + 2 + NUMBER_TOKEN.len() as u64 + 5 + text.0
}

/// A `fmt::Write` that only counts the bytes written, so a number's text is
/// measured without being built.
struct TextLen(u64);

impl fmt::Write for TextLen {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 += text.len() as u64;
        Ok(())
    }
}

/// Decimal digits of an array index, which is its BSON key.
fn decimal_digits(index: usize) -> usize {
    index.checked_ilog10().map_or(1, |log| log as usize + 1)
}

/// A transaction Bundle refused for carrying more payload than the configured
/// limit (#2351).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PayloadLimitRefusal {
    /// The Bundle's counted payload, in bytes.
    pub(super) payload_bytes: u64,
    /// The configured limit, in bytes.
    pub(super) limit_bytes: u64,
}

impl PayloadLimitRefusal {
    /// `None` when no limit is set (`None` or `Some(0)`: nothing is counted) or
    /// when the payload is at or under it.
    pub(super) fn check(limit_bytes: Option<u64>, entries: &[BundleEntry]) -> Option<Self> {
        let limit_bytes = limit_bytes.filter(|limit| *limit > 0)?;
        let payload_bytes = bundle_payload_bytes(entries);
        (payload_bytes > limit_bytes).then_some(Self {
            payload_bytes,
            limit_bytes,
        })
    }

    /// The error the Bundle ends with. `entries` is the Bundle's entry count.
    pub(super) fn into_error(self, entries: usize) -> TransactionError {
        TransactionError::TooLargeForCache {
            cause: TooLargeForCacheCause::PayloadOverLimit {
                payload_bytes: self.payload_bytes,
                limit_bytes: self.limit_bytes,
            },
            entries,
            reason: format!(
                "Refused before admission: the Bundle's create and update entries carry {} bytes \
                 of resource payload (BSON), over the configured limit of {} bytes; no session or \
                 transaction was started",
                self.payload_bytes, self.limit_bytes
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use helios_fhir::FhirVersion;
    use mongodb::bson::RawDocumentBuf;
    use serde_json::json;

    use super::super::storage::value_to_document;
    use super::super::{MongoBackend, MongoBackendConfig};
    use super::*;
    use crate::core::BundleProvider;
    use crate::tenant::{TenantContext, TenantId, TenantPermissions};

    fn entry(method: BundleMethod, resource: Option<Value>) -> BundleEntry {
        BundleEntry {
            method,
            url: "Patient".to_string(),
            resource,
            if_match: None,
            if_none_match: None,
            if_none_exist: None,
            full_url: None,
            criteria: None,
        }
    }

    /// The size of `value` as the driver encodes it for the `resources` write.
    fn driver_len(value: &Value) -> u64 {
        let document = value_to_document(value).expect("a JSON object is a BSON document");
        RawDocumentBuf::from_document(&document)
            .expect("the document encodes")
            .as_bytes()
            .len() as u64
    }

    #[test]
    fn bson_document_len_matches_the_driver_encoding() {
        let many_numbers: Vec<Value> = (0..=120).map(|n| json!(n)).collect();
        let fixtures = vec![
            json!({
                "resourceType": "Patient",
                "id": "pl-1",
                "meta": { "versionId": "1", "lastUpdated": "2026-01-02T03:04:05.678Z" },
                "extension": [
                    { "url": "http://example.org/a", "valueDecimal": 1.5 },
                    { "url": "http://example.org/b", "valueInteger": -3 }
                ],
                "identifier": [{ "system": "urn:oid:1.2.3", "value": "MRN-0001" }],
                "active": true,
                "name": [
                    { "family": "Zoë 日本", "given": ["Anna", "Maria"] },
                    { "use": "nickname", "given": ["Z"] }
                ],
                "telecom": [
                    { "system": "phone", "value": "+1 555 0100" },
                    { "system": "email", "value": "z@example.org" }
                ],
                "birthDate": "1980-05-17",
                "multipleBirthBoolean": false,
                "deceasedBoolean": null
            }),
            // 1-, 2- and 3-digit array keys.
            json!({ "resourceType": "Basic", "n": many_numbers }),
            json!({
                "s": "", "z": null, "t": true, "f": false, "o": {}, "e": [],
                "big": 9_223_372_036_854_775_807i64, "neg": -1, "x": 0.1
            }),
            json!({ "a": { "b": [{ "c": [[1, 2], [3]] }] } }),
            // Numbers as parsed keep their literal text, which sets their size.
            serde_json::from_str(
                r#"{"a":1.50,"b":1e3,"c":-0.0,"d":12345678901234567890123,"e":[0,10,100.25]}"#,
            )
            .unwrap(),
            json!({}),
        ];
        for fixture in &fixtures {
            let object = fixture.as_object().expect("fixtures are objects");
            assert_eq!(bson_document_len(object), driver_len(fixture), "{fixture}");
        }
    }

    #[test]
    fn bundle_payload_bytes_counts_post_and_put_resources_only() {
        let a = json!({ "resourceType": "Patient", "id": "a", "active": true });
        let b = json!({ "resourceType": "Patient", "id": "b", "name": [{ "family": "Bee" }] });
        let patch = json!({
            "resourceType": "Parameters",
            "parameter": [{ "name": "operation", "part": [
                { "name": "value", "valueString": "x".repeat(10_000) }
            ]}]
        });
        let entries = vec![
            entry(BundleMethod::Post, Some(a.clone())),
            entry(BundleMethod::Put, Some(b.clone())),
            entry(BundleMethod::Patch, Some(patch)),
            entry(BundleMethod::Delete, None),
            entry(BundleMethod::Get, None),
            entry(BundleMethod::Post, None),
        ];
        assert_eq!(
            bundle_payload_bytes(&entries),
            driver_len(&a) + driver_len(&b)
        );
        assert_eq!(bundle_payload_bytes(&[]), 0);
    }

    fn big_bundle() -> Vec<BundleEntry> {
        (0..10)
            .map(|n| {
                entry(
                    BundleMethod::Post,
                    Some(
                        json!({ "resourceType": "Patient", "id": format!("p{n}"), "active": true }),
                    ),
                )
            })
            .collect()
    }

    #[test]
    fn an_unset_or_zero_limit_refuses_nothing() {
        let entries = big_bundle();
        assert!(bundle_payload_bytes(&entries) > 0);
        assert_eq!(PayloadLimitRefusal::check(None, &entries), None);
        assert_eq!(PayloadLimitRefusal::check(Some(0), &entries), None);
    }

    #[test]
    fn the_limit_refuses_only_a_payload_above_it() {
        let entries = big_bundle();
        let payload = bundle_payload_bytes(&entries);
        assert_eq!(PayloadLimitRefusal::check(Some(payload), &entries), None);
        assert_eq!(
            PayloadLimitRefusal::check(Some(payload + 1), &entries),
            None
        );
        assert_eq!(
            PayloadLimitRefusal::check(Some(payload - 1), &entries),
            Some(PayloadLimitRefusal {
                payload_bytes: payload,
                limit_bytes: payload - 1,
            })
        );
    }

    #[test]
    fn the_refusal_is_too_large_for_cache_with_its_own_cause() {
        let error = PayloadLimitRefusal {
            payload_bytes: 61_234_567,
            limit_bytes: 50_000_000,
        }
        .into_error(22);
        let rendered = error.to_string();
        let TransactionError::TooLargeForCache {
            cause,
            entries,
            reason,
        } = error
        else {
            panic!("expected TooLargeForCache, got {error:?}");
        };
        assert_eq!(
            cause,
            TooLargeForCacheCause::PayloadOverLimit {
                payload_bytes: 61_234_567,
                limit_bytes: 50_000_000,
            }
        );
        assert_eq!(entries, 22);
        assert!(reason.starts_with("Refused before admission"), "{reason}");
        assert!(reason.contains("61234567"), "{reason}");
        assert!(reason.contains("50000000"), "{reason}");
        assert!(
            reason.contains("no session or transaction was started"),
            "{reason}"
        );
        assert!(
            rendered.contains("payload over the configured limit"),
            "{rendered}"
        );
    }

    /// #2351: the refusal comes before the admission. The whole room is held,
    /// so a Bundle that waits for it cannot answer; the over-limit one answers
    /// at once, and the control under the limit does not answer within 200 ms.
    /// A server is not needed (the backend is lazy, nothing is sent to the
    /// unused address).
    #[tokio::test]
    async fn a_bundle_over_the_limit_is_refused_while_the_admission_room_is_full() {
        let over = big_bundle();
        let under = vec![entry(BundleMethod::Post, Some(json!({})))];
        let payload = bundle_payload_bytes(&over);
        let limit = payload - 1;
        assert!(bundle_payload_bytes(&under) <= limit);

        let backend = MongoBackend::new(MongoBackendConfig {
            connection_string: "mongodb://127.0.0.1:9".into(),
            max_concurrent_transaction_bundles: 1,
            transaction_bundle_weight_entries: 1,
            transaction_bundle_max_payload_bytes: Some(limit),
            ..Default::default()
        })
        .expect("a lazy backend needs no connection");
        let tenant = TenantContext::new(
            TenantId::new("payload-limit"),
            TenantPermissions::full_access(),
        );

        let held = backend.transaction_bundle_gate().admit(1).await;
        assert!(held.is_gated() && held.holds_whole_room());

        let refused = tokio::time::timeout(
            Duration::from_secs(2),
            backend.process_transaction(&tenant, over, FhirVersion::default()),
        )
        .await
        .expect("refused without waiting for the admission room");
        match refused {
            Err(TransactionError::TooLargeForCache {
                cause:
                    TooLargeForCacheCause::PayloadOverLimit {
                        payload_bytes,
                        limit_bytes,
                    },
                entries,
                ..
            }) => {
                assert_eq!(payload_bytes, payload);
                assert_eq!(limit_bytes, limit);
                assert_eq!(entries, 10);
            }
            other => panic!("expected the payload refusal, got {other:?}"),
        }

        // Control: under the limit the Bundle does not answer at once. It waits
        // for the held room (and past the room it would need the unreachable
        // server), so this only shows that the ordinary path does not answer
        // fast; the 2 s bound above is what pins the refusal before the
        // admission.
        let waiting = tokio::time::timeout(
            Duration::from_millis(200),
            backend.process_transaction(&tenant, under, FhirVersion::default()),
        )
        .await;
        assert!(
            waiting.is_err(),
            "an under-limit Bundle does not answer at once"
        );

        drop(held);
    }
}
