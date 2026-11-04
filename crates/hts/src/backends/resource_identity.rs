//! Public identities are separate from normalized keys and canonical versions.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

#[derive(Clone, Copy)]
pub(crate) enum ResourceKind {
    CodeSystem,
    ValueSet,
    ConceptMap,
}

impl ResourceKind {
    pub const VERSIONED_KINDS: [Self; 2] = [Self::CodeSystem, Self::ValueSet];

    pub fn from_resource_type(resource_type: &str) -> Option<Self> {
        match resource_type {
            "CodeSystem" => Some(Self::CodeSystem),
            "ValueSet" => Some(Self::ValueSet),
            "ConceptMap" => Some(Self::ConceptMap),
            _ => None,
        }
    }

    pub fn table(self) -> &'static str {
        match self {
            Self::CodeSystem => "code_systems",
            Self::ValueSet => "value_sets",
            Self::ConceptMap => "concept_maps",
        }
    }

    #[cfg(feature = "postgres")]
    pub fn postgres_identity_expression(self) -> &'static str {
        match self {
            Self::ConceptMap => "id",
            Self::CodeSystem | Self::ValueSet => "COALESCE(resource_json->>'id', id)",
        }
    }

    #[cfg(feature = "sqlite")]
    pub fn sqlite_identity_expression(self) -> &'static str {
        match self {
            Self::ConceptMap => "id",
            Self::CodeSystem | Self::ValueSet => {
                "COALESCE(json_extract(resource_json, '$.id'), id)"
            }
        }
    }

    pub fn resource_type(self) -> &'static str {
        match self {
            Self::CodeSystem => "CodeSystem",
            Self::ValueSet => "ValueSet",
            Self::ConceptMap => "ConceptMap",
        }
    }
}

pub(crate) fn valid_logical_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
}

/// A persisted resource can lend its ID only to its own normalized row.
/// Deleted resources reserve their logical ID; reimport may use a new ID.
#[cfg(feature = "sqlite")]
pub(crate) fn matches_persisted_resource(
    resource: &Value,
    deleted: bool,
    url: &str,
    version: Option<&str>,
) -> bool {
    persisted_canonical_matches(
        resource.get("url").and_then(Value::as_str),
        resource.get("version").and_then(Value::as_str),
        deleted,
        url,
        version,
    )
}

/// [`matches_persisted_resource`] over a persisted resource's string `url` and
/// `version`, for callers that read them without the resource body.
pub(crate) fn persisted_canonical_matches(
    persisted_url: Option<&str>,
    persisted_version: Option<&str>,
    deleted: bool,
    url: &str,
    version: Option<&str>,
) -> bool {
    !deleted
        && persisted_url == Some(url)
        && persisted_version.unwrap_or("") == version.unwrap_or("")
}

/// Identity inputs read while backfilling a legacy normalized row.
pub(crate) struct ImportIdentity {
    pub storage_id: String,
    pub url: String,
    pub version: Option<String>,
    pub source_id: Option<String>,
    pub resource_is_null: bool,
}

/// The backends historically exposed different IDs for imported JSON.
pub(crate) enum LegacyIdentitySource {
    #[cfg(feature = "postgres")]
    StorageKey,
    #[cfg(feature = "sqlite")]
    ResourceJson,
}

/// Live CRUD owners indexed once during migration, rather than scanning their
/// JSON for every normalized resource. Deleted rows remain reserved by allocate.
#[derive(Default)]
pub(crate) struct MigrationOwners {
    canonical: BTreeMap<(String, String), Vec<String>>,
    reserved: BTreeMap<String, Option<(String, String)>>,
    assigned: BTreeSet<String>,
}

impl MigrationOwners {
    pub fn insert(
        &mut self,
        id: String,
        url: Option<String>,
        version: Option<String>,
        deleted: bool,
    ) {
        let canonical = url.map(|url| (url, version.unwrap_or_default()));
        if !deleted
            && valid_logical_id(&id)
            && let Some(key) = &canonical
        {
            self.canonical
                .entry(key.clone())
                .or_default()
                .push(id.clone());
        }
        self.reserved
            .insert(id, if deleted { None } else { canonical });
    }

    /// Plan against assigned IDs, never against not-yet-repaired legacy JSON.
    /// This preserves permutations of valid public IDs without temporary writes.
    pub fn allocate(&mut self, row: &ImportIdentity, source: LegacyIdentitySource) -> String {
        let preferred = self.preferred_id(row, source);
        let mut candidate = if valid_logical_id(preferred) {
            preferred.to_owned()
        } else {
            uuid::Uuid::new_v4().to_string()
        };
        let canonical = (row.url.clone(), row.version.clone().unwrap_or_default());
        while self.assigned.contains(&candidate)
            || self
                .reserved
                .get(&candidate)
                .is_some_and(|owner| owner.as_ref() != Some(&canonical))
        {
            candidate = uuid::Uuid::new_v4().to_string();
        }
        self.assigned.insert(candidate.clone());
        candidate
    }

    fn preferred_id<'a>(
        &'a self,
        row: &'a ImportIdentity,
        source: LegacyIdentitySource,
    ) -> &'a str {
        if let Some(owners) = self
            .canonical
            .get(&(row.url.clone(), row.version.clone().unwrap_or_default()))
        {
            // Prefer the owner represented by the storage key, even if a later
            // canonical upsert changed the source JSON's id.
            let matching = owners.iter().find(|id| {
                row.storage_id == **id
                    || row
                        .version
                        .as_ref()
                        .is_some_and(|version| row.storage_id == format!("{id}|{version}"))
            });
            if let Some(id) = matching
                .or_else(|| owners.iter().find(|id| row.source_id.as_ref() == Some(id)))
                .or_else(|| owners.iter().min())
            {
                return id;
            }
        }
        match source {
            #[cfg(feature = "sqlite")]
            LegacyIdentitySource::ResourceJson => {
                if let Some(id) = row.source_id.as_deref().filter(|id| valid_logical_id(id)) {
                    return id;
                }
            }
            #[cfg(feature = "postgres")]
            LegacyIdentitySource::StorageKey => {}
        }
        if valid_logical_id(&row.storage_id) {
            &row.storage_id
        } else {
            row.source_id.as_deref().unwrap_or(&row.storage_id)
        }
    }
}

/// Metadata and optional original JSON for one normalized resource.
pub(crate) struct ResourceRecord {
    pub url: String,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: String,
    pub resource: Option<Value>,
    pub compose: Option<String>,
}

impl ResourceRecord {
    pub fn into_resource(
        self,
        kind: ResourceKind,
        id: &str,
    ) -> Result<Value, crate::error::HtsError> {
        let mut resource = self.resource.unwrap_or_else(|| {
            let mut resource = serde_json::json!({
                "resourceType": kind.resource_type(), "id": id,
                "url": self.url, "status": self.status,
            });
            for (field, value) in [
                ("version", self.version),
                ("name", self.name),
                ("title", self.title),
            ] {
                if let Some(value) = value {
                    resource[field] = Value::String(value);
                }
            }
            resource
        });
        resource["id"] = Value::String(id.to_owned());
        if resource.get("compose").is_none()
            && let Some(compose) = self.compose
        {
            resource["compose"] = serde_json::from_str(&compose)
                .map_err(|error| crate::error::HtsError::StorageError(error.to_string()))?;
        }
        Ok(resource)
    }
}
