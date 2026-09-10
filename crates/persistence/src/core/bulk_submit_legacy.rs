//! Backend-independent classification of legacy bulk-submit publications.

use serde_json::Value;

pub(crate) const LEGACY_REASON_NULL_URL: &str = "legacy-null-url";
pub(crate) const LEGACY_REASON_UNMATCHED_URL: &str = "legacy-unmatched-url";
pub(crate) const LEGACY_REASON_AMBIGUOUS_URL: &str = "legacy-ambiguous-url";
pub(crate) const LEGACY_REASON_TOKEN_MISMATCH: &str = "legacy-token-mismatch";
pub(crate) const LEGACY_REASON_NONTERMINAL: &str = "legacy-nonterminal";
pub(crate) const LEGACY_REASON_EXACT_DUPLICATE: &str = "legacy-exact-duplicate";
pub(crate) const LEGACY_REASON_INCOMPLETE_GENERATION: &str = "legacy-incomplete-generation";
pub(crate) const LEGACY_REASON_CONFLICTING_DUPLICATE: &str = "legacy-conflicting-duplicate";
pub(crate) const LEGACY_REASON_INVALID_DOMAIN: &str = "legacy-invalid-domain";

#[derive(Clone, Debug)]
pub(crate) struct LegacyManifest {
    pub(crate) tenant_id: String,
    pub(crate) submitter: String,
    pub(crate) submission_id: String,
    pub(crate) manifest_id: String,
    pub(crate) manifest_url: Option<String>,
    pub(crate) status: String,
    pub(crate) fencing_token: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LegacyArtifactMetadata {
    pub(crate) manifest_url: Option<String>,
    pub(crate) file_type: String,
    pub(crate) resource_type: Option<String>,
    pub(crate) part_index: i64,
    pub(crate) file_path: String,
    pub(crate) line_count: i64,
    pub(crate) byte_count: i64,
    pub(crate) count_severity: Option<Value>,
    pub(crate) count_severity_invalid: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct LegacyFile {
    pub(crate) id: i64,
    pub(crate) tenant_id: String,
    pub(crate) submitter: String,
    pub(crate) submission_id: String,
    pub(crate) metadata: LegacyArtifactMetadata,
    pub(crate) fencing_token: i64,
}

impl LegacyFile {
    fn submission_key(&self) -> (&str, &str, &str) {
        (
            self.tenant_id.as_str(),
            self.submitter.as_str(),
            self.submission_id.as_str(),
        )
    }

    fn has_valid_domain_metadata(&self) -> bool {
        let metadata = &self.metadata;
        matches!(metadata.file_type.as_str(), "output" | "error" | "deleted")
            && !metadata.file_path.is_empty()
            && (0..=i32::MAX as i64).contains(&metadata.part_index)
            && metadata.line_count >= 0
            && metadata.byte_count >= 0
            && self.fencing_token >= 0
            && !metadata.count_severity_invalid
            && metadata.count_severity.as_ref().is_none_or(|severity| {
                severity
                    .as_object()
                    .is_some_and(|counts| counts.values().all(|count| count.as_u64().is_some()))
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LegacyManifestKey<'a> {
    tenant_id: &'a str,
    submitter: &'a str,
    submission_id: &'a str,
    manifest_id: &'a str,
    fencing_token: i64,
}

impl<'a> From<&'a LegacyManifest> for LegacyManifestKey<'a> {
    fn from(manifest: &'a LegacyManifest) -> Self {
        Self {
            tenant_id: manifest.tenant_id.as_str(),
            submitter: manifest.submitter.as_str(),
            submission_id: manifest.submission_id.as_str(),
            manifest_id: manifest.manifest_id.as_str(),
            fencing_token: manifest.fencing_token,
        }
    }
}

#[derive(Debug)]
pub(crate) enum LegacyPublicationAction {
    Exclude { id: i64, reason: &'static str },
    Assign { id: i64, manifest_id: String },
    Publish { manifest: LegacyManifest },
}

fn url_matches(manifest: &LegacyManifest, url: Option<&str>) -> bool {
    match (manifest.manifest_url.as_deref(), url) {
        (Some(manifest_url), Some(file_url)) => manifest_url == file_url,
        _ => false,
    }
}

pub(crate) fn classify_legacy_publications(
    manifests: &[LegacyManifest],
    files: &[LegacyFile],
) -> Vec<LegacyPublicationAction> {
    let mut actions = Vec::new();

    let mut manifests_by_submission: std::collections::HashMap<
        (&str, &str, &str),
        Vec<&LegacyManifest>,
    > = std::collections::HashMap::new();
    for manifest in manifests {
        manifests_by_submission
            .entry((
                manifest.tenant_id.as_str(),
                manifest.submitter.as_str(),
                manifest.submission_id.as_str(),
            ))
            .or_default()
            .push(manifest);
    }

    let row_reasons: std::collections::HashMap<i64, &'static str> = files
        .iter()
        .map(|file| {
            let exact_matches = file
                .metadata
                .manifest_url
                .as_deref()
                .map(|file_url| {
                    manifests_by_submission
                        .get(&file.submission_key())
                        .map(|manifests| {
                            manifests
                                .iter()
                                .filter(|manifest| url_matches(manifest, Some(file_url)))
                                .count()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            let same_token_matches = file
                .metadata
                .manifest_url
                .as_deref()
                .map(|file_url| {
                    manifests_by_submission
                        .get(&file.submission_key())
                        .map(|manifests| {
                            manifests
                                .iter()
                                .filter(|manifest| {
                                    url_matches(manifest, Some(file_url))
                                        && file.fencing_token == manifest.fencing_token
                                })
                                .count()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            (
                file.id,
                base_reason(file, exact_matches, same_token_matches),
            )
        })
        .collect();
    let mut candidate_rows: std::collections::HashMap<LegacyManifestKey<'_>, Vec<&LegacyFile>> =
        std::collections::HashMap::new();

    for file in files {
        let candidates_for_token =
            |manifest: &LegacyManifest| file.fencing_token == manifest.fencing_token;

        let associations: Vec<&LegacyManifest> = match file.metadata.manifest_url.as_deref() {
            Some(file_url) => manifests_by_submission
                .get(&file.submission_key())
                .map(|manifests| {
                    manifests
                        .iter()
                        .copied()
                        .filter(|manifest| url_matches(manifest, Some(file_url)))
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };

        match associations.as_slice() {
            [] => {
                // An explicit URL that matches no manifest is not proof that it
                // belonged to another manifest; conservatively make it a
                // possible member of every same-token generation in its
                // submission. A NULL URL gets the same treatment.
                for manifest in manifests_by_submission
                    .get(&file.submission_key())
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .filter(|manifest| candidates_for_token(manifest))
                {
                    candidate_rows
                        .entry(manifest.into())
                        .or_default()
                        .push(file);
                }
            }
            [manifest] if candidates_for_token(manifest) => {
                candidate_rows
                    .entry((*manifest).into())
                    .or_default()
                    .push(file);
            }
            _ => {
                for manifest in associations
                    .iter()
                    .copied()
                    .filter(|manifest| candidates_for_token(manifest))
                {
                    candidate_rows
                        .entry(manifest.into())
                        .or_default()
                        .push(file);
                }
            }
        }
    }

    let mut candidate_ids = std::collections::HashSet::new();
    for rows in candidate_rows.values() {
        for row in rows {
            candidate_ids.insert(row.id);
        }
    }
    for file in files {
        if !candidate_ids.contains(&file.id) {
            let reason = row_reasons
                .get(&file.id)
                .copied()
                .expect("base reason for every candidate");
            actions.push(LegacyPublicationAction::Exclude {
                id: file.id,
                reason,
            });
        }
    }

    for manifest in manifests {
        let rows = candidate_rows
            .get(&LegacyManifestKey::from(manifest))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let terminal = matches!(manifest.status.as_str(), "completed" | "failed");

        if rows.is_empty() {
            // Legacy rows have no publication seal. An absent artifact set
            // cannot establish that an empty publication was committed.
            continue;
        }

        let incomplete = rows.iter().any(|row| {
            let url = row.metadata.manifest_url.as_deref();
            let exact_matches = manifests_by_submission
                .get(&row.submission_key())
                .map(|manifests| {
                    manifests
                        .iter()
                        .filter(|candidate| url_matches(candidate, url))
                        .count()
                })
                .unwrap_or(0);
            url.is_none() || exact_matches != 1
        });
        let invalid = rows.iter().any(|row| !row.has_valid_domain_metadata());
        if invalid || !terminal {
            let reason = if invalid {
                LEGACY_REASON_INVALID_DOMAIN
            } else {
                LEGACY_REASON_NONTERMINAL
            };
            for row in rows {
                actions.push(LegacyPublicationAction::Exclude { id: row.id, reason });
            }
            continue;
        }

        let mut grouped: std::collections::HashMap<
            (String, Option<String>, i64),
            Vec<&LegacyFile>,
        > = std::collections::HashMap::new();
        for row in rows {
            grouped
                .entry((
                    row.metadata.file_type.clone(),
                    row.metadata.resource_type.clone(),
                    row.metadata.part_index,
                ))
                .or_default()
                .push(row);
        }

        let mut conflicting = false;
        let mut representatives: Vec<&LegacyFile> = Vec::new();
        let mut exact_duplicates: Vec<&LegacyFile> = Vec::new();
        for mut group in grouped.into_values() {
            group.sort_by_key(|row| row.id);
            if group.len() == 1 {
                representatives.push(group[0]);
            } else if group
                .windows(2)
                .all(|pair| pair[0].metadata == pair[1].metadata)
            {
                representatives.push(group[0]);
                exact_duplicates.extend(&group[1..]);
            } else {
                conflicting = true;
                break;
            }
        }

        if incomplete || conflicting {
            for row in rows {
                let reason = if conflicting {
                    LEGACY_REASON_CONFLICTING_DUPLICATE
                } else if row_reasons.get(&row.id).copied() == Some(LEGACY_REASON_NONTERMINAL) {
                    unreachable!("terminal manifest rows cannot be nonterminal");
                } else if row_reasons.get(&row.id).copied() == Some(LEGACY_REASON_NULL_URL)
                    || row_reasons.get(&row.id).copied() == Some(LEGACY_REASON_UNMATCHED_URL)
                    || row_reasons.get(&row.id).copied() == Some(LEGACY_REASON_AMBIGUOUS_URL)
                {
                    row_reasons.get(&row.id).copied().expect("base reason")
                } else {
                    LEGACY_REASON_INCOMPLETE_GENERATION
                };
                actions.push(LegacyPublicationAction::Exclude { id: row.id, reason });
            }
            continue;
        }

        for row in &exact_duplicates {
            actions.push(LegacyPublicationAction::Exclude {
                id: row.id,
                reason: LEGACY_REASON_EXACT_DUPLICATE,
            });
        }
        for row in representatives {
            actions.push(LegacyPublicationAction::Assign {
                id: row.id,
                manifest_id: manifest.manifest_id.clone(),
            });
        }
        actions.push(LegacyPublicationAction::Publish {
            manifest: manifest.clone(),
        });
    }

    actions
}

fn base_reason(file: &LegacyFile, exact_matches: usize, same_token_matches: usize) -> &'static str {
    match file.metadata.manifest_url.as_deref() {
        None => LEGACY_REASON_NULL_URL,
        Some(_) if exact_matches == 0 => LEGACY_REASON_UNMATCHED_URL,
        Some(_) if exact_matches > 1 => LEGACY_REASON_AMBIGUOUS_URL,
        Some(_) if same_token_matches == 1 => LEGACY_REASON_INCOMPLETE_GENERATION,
        Some(_) => LEGACY_REASON_TOKEN_MISMATCH,
    }
}
