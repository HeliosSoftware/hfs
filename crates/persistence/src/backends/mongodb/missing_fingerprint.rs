//! Definition fingerprints for `missing_params` metadata (#1999).
//!
//! Every writer stamps `missing_params_fp` next to `missing_params`: the
//! fingerprint of the definitions the array was computed from
//! ([`SearchParameterExtractor::missing_fingerprint`]). The first time a
//! process stamps a fingerprint it records the fingerprint's per-parameter
//! digests in [`FINGERPRINT_COLLECTION`], so a search on a node holding other
//! definitions can tell which parameters those arrays still describe.
//!
//! Before each `:missing=true` search or count, one probe of
//! `idx_resources_missing_params_fp` finds the stale stamps among the live
//! resources of the tenant and type, and the arrays without a stamp (written
//! before #1999 or by a node that predates it). A parameter keeps the
//! metadata route only when every stale stamp recorded an identical
//! definition for it; anything unknown, unrecorded or unstamped sends it to
//! the value-index route, which answers correctly from the search rows.
//!
//! [`SearchParameterExtractor::missing_fingerprint`]: crate::search::SearchParameterExtractor

use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::Database;
use mongodb::bson::{Bson, Document, doc};

use crate::search::extractor::MissingFingerprint;
use crate::types::{SearchModifier, SearchQuery};

use super::MongoBackend;
use super::schema::RESOURCES_MISSING_FP_INDEX;

/// `{ _id: fingerprint, params: [{ code, digest }] }`, written once per
/// fingerprint and never changed: a fingerprint names one set of definitions.
pub(crate) const FINGERPRINT_COLLECTION: &str = "missing_params_fingerprints";

/// More distinct stale stamps than this on one type distrusts every
/// parameter instead of reading that many catalog entries per search.
const MAX_STALE_FINGERPRINTS: usize = 32;

/// Which parameters of a tenant's type may answer from `missing_params`,
/// as of the probe that computed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MissingFreshness {
    /// Every live array carries the current fingerprint.
    All,
    /// Only these codes: their definition is identical under every stamp
    /// found. Empty when an array is unstamped or a stamp is unrecorded.
    Only(HashSet<String>),
}

impl MissingFreshness {
    pub(super) fn allows(&self, code: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(codes) => codes.contains(code),
        }
    }
}

/// The parameters whose digest is the same in `current` and every one of
/// `stale` (recorded `code → digest` maps).
fn fresh_codes(current: &MissingFingerprint, stale: &[HashMap<String, String>]) -> HashSet<String> {
    current
        .params
        .iter()
        .filter(|(code, digest)| {
            stale
                .iter()
                .all(|recorded| recorded.get(*code) == Some(*digest))
        })
        .map(|(code, _)| code.clone())
        .collect()
}

fn catalog_entry(fingerprint: &MissingFingerprint) -> Document {
    let params: Vec<Document> = fingerprint
        .params
        .iter()
        .map(|(code, digest)| doc! { "code": code, "digest": digest })
        .collect();
    doc! { "params": params }
}

fn parse_catalog_entry(entry: &Document) -> Option<HashMap<String, String>> {
    entry
        .get_array("params")
        .ok()?
        .iter()
        .map(|param| {
            let param = param.as_document()?;
            Some((
                param.get_str("code").ok()?.to_owned(),
                param.get_str("digest").ok()?.to_owned(),
            ))
        })
        .collect()
}

async fn record(
    database: &Database,
    fingerprint: &MissingFingerprint,
) -> mongodb::error::Result<()> {
    database
        .collection::<Document>(FINGERPRINT_COLLECTION)
        .update_one(
            doc! { "_id": &fingerprint.fingerprint },
            doc! { "$setOnInsert": catalog_entry(fingerprint) },
        )
        .upsert(true)
        .await
        .map(|_| ())
}

fn is_missing_true(query: &SearchQuery) -> bool {
    query.parameters.iter().any(|param| {
        matches!(param.modifier, Some(SearchModifier::Missing))
            && param
                .values
                .first()
                .is_some_and(|value| value.value == "true")
    })
}

impl MongoBackend {
    /// The fingerprint a writer stamps for `resource_type`, recorded in the
    /// catalog the first time this process uses it. The record is written in
    /// the background: until it lands, other definitions' searches distrust
    /// the stamp, which only costs them the value-index route.
    pub(super) fn stamp_missing_fingerprint(&self, tenant_id: &str, resource_type: &str) -> String {
        let fingerprint = self
            .tenant_extractor(tenant_id)
            .missing_fingerprint(resource_type);
        if !self
            .recorded_missing_fps
            .lock()
            .insert(fingerprint.fingerprint.clone())
        {
            return fingerprint.fingerprint;
        }
        // Extraction may run on the `$reindex` rayon pool, outside the
        // runtime; spawn on the runtime a writer last ran on.
        let runtime = match tokio::runtime::Handle::try_current() {
            Ok(handle) => Some(self.missing_fp_runtime.get_or_init(|| handle).clone()),
            Err(_) => self.missing_fp_runtime.get().cloned(),
        };
        let client = self.client_cell().get().cloned();
        let (Some(runtime), Some(client)) = (runtime, client) else {
            self.recorded_missing_fps
                .lock()
                .remove(&fingerprint.fingerprint);
            return fingerprint.fingerprint;
        };
        let database = client.database(&self.config().database_name);
        let stamp = fingerprint.fingerprint.clone();
        let recorded = std::sync::Arc::clone(&self.recorded_missing_fps);
        runtime.spawn(async move {
            if let Err(error) = record(&database, &fingerprint).await {
                tracing::warn!(
                    %error,
                    fingerprint = %fingerprint.fingerprint,
                    "Failed to record a missing_params fingerprint; searches under other \
                     definitions use the value-index route for its resources"
                );
                recorded.lock().remove(&fingerprint.fingerprint);
            }
        });
        stamp
    }

    /// Which codes of `resource_type` may read `missing_params`: only those
    /// this registry's probe found fresh. A missing or outdated probe result
    /// (a search path that did not probe, or a definition change since) is
    /// `None`, which trusts no code.
    pub(super) fn missing_freshness(
        &self,
        tenant_id: &str,
        resource_type: &str,
    ) -> Option<MissingFreshness> {
        let key = (tenant_id.to_string(), resource_type.to_string());
        let (fingerprint, freshness) = self.missing_meta_fresh.read().get(&key).cloned()?;
        (fingerprint
            == self
                .tenant_extractor(tenant_id)
                .missing_fingerprint(resource_type)
                .fingerprint)
            .then_some(freshness)
    }

    /// Probes `idx_resources_missing_params_fp` before a `:missing=true`
    /// search and caches which parameters may read `missing_params`.
    pub(super) async fn refresh_missing_metadata_freshness(
        &self,
        tenant_id: &str,
        query: &SearchQuery,
    ) {
        if !is_missing_true(query) || !self.missing_params_index_ready() {
            return;
        }
        let current = self
            .tenant_extractor(tenant_id)
            .missing_fingerprint(&query.resource_type);
        let freshness = match self
            .probe_missing_freshness(tenant_id, query, &current)
            .await
        {
            Ok(freshness) => freshness,
            Err(error) => {
                tracing::warn!(
                    %error,
                    tenant_id,
                    resource_type = %query.resource_type,
                    "Missing-metadata staleness probe failed; using the value-index route"
                );
                MissingFreshness::Only(HashSet::new())
            }
        };
        self.missing_meta_fresh.write().insert(
            (tenant_id.to_string(), query.resource_type.clone()),
            (current.fingerprint, freshness),
        );
    }

    async fn probe_missing_freshness(
        &self,
        tenant_id: &str,
        query: &SearchQuery,
        current: &MissingFingerprint,
    ) -> crate::error::StorageResult<MissingFreshness> {
        let database = self.get_database().await?;
        // Searches record their own definitions too, so a node that stamps
        // only from the rayon pool still has its fingerprint in the catalog.
        if !self
            .recorded_missing_fps
            .lock()
            .contains(&current.fingerprint)
        {
            record(&database, current).await?;
            self.recorded_missing_fps
                .lock()
                .insert(current.fingerprint.clone());
        }
        let resources = database.collection::<Document>(Self::RESOURCES_COLLECTION);
        // The index is partial on `missing_params` existing, so arrays
        // without a stamp are its null keys: one seek finds the first.
        let scope = doc! {
            "tenant_id": tenant_id,
            "resource_type": &query.resource_type,
            "is_deleted": false,
            "missing_params": { "$exists": true },
        };
        let mut unstamped = scope.clone();
        unstamped.insert("missing_params_fp", doc! { "$exists": false });
        if resources
            .find_one(unstamped)
            .hint(mongodb::options::Hint::Name(
                RESOURCES_MISSING_FP_INDEX.to_string(),
            ))
            .projection(doc! { "_id": 1 })
            .await?
            .is_some()
        {
            return Ok(MissingFreshness::Only(HashSet::new()));
        }
        // The distinct stamps, one covered seek each (a `$group` would read
        // every stale key, and after a definition change that is the type).
        let hint = mongodb::options::Hint::Name(RESOURCES_MISSING_FP_INDEX.to_string());
        let order = doc! {
            "tenant_id": 1_i32,
            "resource_type": 1_i32,
            "is_deleted": 1_i32,
            "missing_params_fp": 1_i32,
        };
        let mut stamps: Vec<Bson> = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut stamp = doc! { "$type": "string" };
            if let Some(after) = &after {
                stamp.insert("$gt", after);
            }
            let mut next = scope.clone();
            next.insert("missing_params_fp", stamp);
            let Some(found) = resources
                .find_one(next)
                .hint(hint.clone())
                .sort(order.clone())
                .projection(doc! { "_id": 0, "missing_params_fp": 1 })
                .await?
                .and_then(|found| found.get_str("missing_params_fp").ok().map(str::to_owned))
            else {
                break;
            };
            if found != current.fingerprint {
                if stamps.len() >= MAX_STALE_FINGERPRINTS {
                    return Ok(MissingFreshness::Only(HashSet::new()));
                }
                stamps.push(Bson::String(found.clone()));
            }
            after = Some(found);
        }
        if stamps.is_empty() {
            return Ok(MissingFreshness::All);
        }
        let recorded: Vec<Document> = database
            .collection::<Document>(FINGERPRINT_COLLECTION)
            .find(doc! { "_id": { "$in": Bson::Array(stamps.clone()) } })
            .await?
            .try_collect()
            .await?;
        let maps: Option<Vec<_>> = recorded.iter().map(parse_catalog_entry).collect();
        match maps {
            Some(maps) if maps.len() == stamps.len() => {
                Ok(MissingFreshness::Only(fresh_codes(current, &maps)))
            }
            _ => Ok(MissingFreshness::Only(HashSet::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(params: &[(&str, &str)]) -> MissingFingerprint {
        MissingFingerprint {
            fingerprint: "current".to_string(),
            params: params
                .iter()
                .map(|(code, digest)| (code.to_string(), digest.to_string()))
                .collect(),
        }
    }

    fn recorded(params: &[(&str, &str)]) -> HashMap<String, String> {
        params
            .iter()
            .map(|(code, digest)| (code.to_string(), digest.to_string()))
            .collect()
    }

    #[test]
    fn only_parameters_unchanged_under_every_stale_stamp_stay_fresh() {
        let current = fingerprint(&[("same", "1"), ("changed", "2"), ("added", "3")]);
        let older = recorded(&[("same", "1"), ("changed", "9"), ("removed", "4")]);
        let oldest = recorded(&[("same", "1"), ("changed", "2")]);
        assert_eq!(
            fresh_codes(&current, std::slice::from_ref(&older)),
            HashSet::from(["same".to_string()])
        );
        assert_eq!(
            fresh_codes(&current, std::slice::from_ref(&oldest)),
            HashSet::from(["same".to_string(), "changed".to_string()])
        );
        assert_eq!(
            fresh_codes(&current, &[older, oldest]),
            HashSet::from(["same".to_string()])
        );
    }

    #[test]
    fn catalog_entries_round_trip_and_reject_malformed_params() {
        let current = fingerprint(&[("a", "1"), ("b.c", "2")]);
        assert_eq!(
            parse_catalog_entry(&catalog_entry(&current)),
            Some(recorded(&[("a", "1"), ("b.c", "2")]))
        );
        assert_eq!(
            parse_catalog_entry(&doc! { "params": [{ "code": "a" }] }),
            None
        );
        assert_eq!(parse_catalog_entry(&doc! {}), None);
    }

    #[test]
    fn freshness_allows_only_listed_codes() {
        assert!(MissingFreshness::All.allows("any"));
        let only = MissingFreshness::Only(HashSet::from(["a".to_string()]));
        assert!(only.allows("a"));
        assert!(!only.allows("b"));
        assert!(!MissingFreshness::Only(HashSet::new()).allows("a"));
    }
}
