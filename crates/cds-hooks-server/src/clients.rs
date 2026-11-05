//! The client registry: which EMR instances may call this service, how each
//! one signs its calls, and which services each one has enabled.
//!
//! A client is one healthcare organization on one EMR instance: one `iss`, or
//! one `iss` and one `tenant` when the issuer signs for several organizations
//! (a cloud EMR with one key and many hospitals). The registry is a YAML file
//! named by `CDS_HOOKS_CLIENTS`; without it the server runs with a built-in
//! development registry in which every caller is the client `dev`.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ClientRegistry {
    /// The `aud` values this service accepts: its own URLs as clients know them.
    #[serde(default)]
    pub audience: Vec<String>,
    /// A client that calls without a JWT is treated as this client. Development only.
    #[serde(default)]
    pub anonymous_client: Option<String>,
    #[serde(default)]
    pub clients: Vec<Client>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Client {
    pub id: String,
    /// The `iss` the client's JWTs carry; the registry key.
    #[serde(default)]
    pub iss: Option<String>,
    /// The `tenant` claim the client's JWTs carry, when its issuer signs for
    /// several organizations. Clients sharing an issuer are told apart by it.
    #[serde(default)]
    pub tenant: Option<String>,
    /// Where the client publishes the keys it signs with.
    #[serde(default)]
    pub jku: Option<String>,
    /// The client's FHIR base; a call naming another `fhirServer` is refused.
    #[serde(default)]
    pub fhir_server: Option<String>,
    /// The service ids this client has enabled.
    #[serde(default)]
    pub services: Vec<String>,
    /// Prefetch templates per service, replacing the service's own for this client.
    #[serde(default)]
    pub prefetch: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing the client registry: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("client registry: {0}")]
    Invalid(String),
}

impl ClientRegistry {
    pub fn from_yaml(text: &str) -> Result<Self, RegistryError> {
        let registry: Self = serde_yaml::from_str(text)?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn load(path: &Path) -> Result<Self, RegistryError> {
        let text = std::fs::read_to_string(path).map_err(|source| RegistryError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_yaml(&text)
    }

    /// The registry the server runs with when none is configured: one client,
    /// `dev`, that every caller is, with every service enabled.
    pub fn development(service_ids: &[String]) -> Self {
        Self {
            audience: Vec::new(),
            anonymous_client: Some("dev".to_string()),
            clients: vec![Client {
                id: "dev".to_string(),
                iss: None,
                tenant: None,
                jku: None,
                fhir_server: None,
                services: service_ids.to_vec(),
                prefetch: HashMap::new(),
            }],
        }
    }

    fn validate(&self) -> Result<(), RegistryError> {
        let mut ids = std::collections::HashSet::new();
        let mut identities = std::collections::HashSet::new();
        let mut issuers: HashMap<&str, (&str, bool)> = HashMap::new();
        for client in &self.clients {
            if client.id.trim().is_empty() {
                return Err(RegistryError::Invalid("a client has no id".into()));
            }
            if !ids.insert(client.id.as_str()) {
                return Err(RegistryError::Invalid(format!(
                    "client id {:?} appears twice",
                    client.id
                )));
            }
            match (&client.iss, &client.jku) {
                (Some(iss), Some(jku)) => {
                    if !identities.insert((iss.as_str(), client.tenant.as_deref())) {
                        return Err(RegistryError::Invalid(match &client.tenant {
                            Some(tenant) => {
                                format!(
                                    "issuer {iss:?} with tenant {tenant:?} belongs to two clients"
                                )
                            }
                            None => format!("issuer {iss:?} belongs to two clients"),
                        }));
                    }
                    let has_tenant = client.tenant.is_some();
                    match issuers.get(iss.as_str()) {
                        Some((other_jku, other_has_tenant)) => {
                            if *other_jku != jku {
                                return Err(RegistryError::Invalid(format!(
                                    "issuer {iss:?} publishes its keys at two different jku"
                                )));
                            }
                            if *other_has_tenant != has_tenant {
                                return Err(RegistryError::Invalid(format!(
                                    "issuer {iss:?} has a client without a tenant and clients with one"
                                )));
                            }
                        }
                        None => {
                            issuers.insert(iss.as_str(), (jku.as_str(), has_tenant));
                        }
                    }
                }
                (None, None) => {
                    if client.tenant.is_some() {
                        return Err(RegistryError::Invalid(format!(
                            "client {:?} has a tenant but no iss",
                            client.id
                        )));
                    }
                }
                _ => {
                    return Err(RegistryError::Invalid(format!(
                        "client {:?} needs both iss and jku, or neither",
                        client.id
                    )));
                }
            }
        }
        if let Some(anon) = &self.anonymous_client
            && !ids.contains(anon.as_str())
        {
            return Err(RegistryError::Invalid(format!(
                "anonymous_client {anon:?} is not a registered client"
            )));
        }
        if self.clients.iter().any(|c| c.iss.is_some()) && self.audience.is_empty() {
            return Err(RegistryError::Invalid(
                "audience is required when any client signs its calls".into(),
            ));
        }
        Ok(())
    }

    /// The client a token's `iss` and `tenant` name. A tenant the issuer has
    /// no client for, or a missing tenant where the issuer's clients have one,
    /// names nobody.
    pub fn by_issuer(&self, iss: &str, tenant: Option<&str>) -> Option<&Client> {
        self.clients
            .iter()
            .find(|c| c.iss.as_deref() == Some(iss) && c.tenant.as_deref() == tenant)
    }

    pub fn by_id(&self, id: &str) -> Option<&Client> {
        self.clients.iter().find(|c| c.id == id)
    }

    /// The service ids any client names that the catalog does not offer.
    pub fn unknown_services<'a>(&'a self, known: &[String]) -> Vec<(&'a str, &'a str)> {
        self.clients
            .iter()
            .flat_map(|c| {
                c.services
                    .iter()
                    .filter(|s| !known.contains(s))
                    .map(move |s| (c.id.as_str(), s.as_str()))
            })
            .collect()
    }
}

impl Client {
    pub fn has_service(&self, id: &str) -> bool {
        self.services.iter().any(|s| s == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_a_registry() {
        let registry = ClientRegistry::from_yaml(
            "audience: [https://cds.example.org]\nclients:\n  - id: helios-emr\n    iss: https://emr.example.org\n    jku: https://emr.example.org/.well-known/jwks.json\n    services: [bcbsm-galleri-crd]\n    prefetch:\n      bcbsm-galleri-crd:\n        patient: Patient/{{context.patientId}}\n",
        )
        .unwrap();
        let client = registry.by_issuer("https://emr.example.org", None).unwrap();
        assert_eq!(client.id, "helios-emr");
        assert!(client.has_service("bcbsm-galleri-crd"));
        assert_eq!(client.prefetch["bcbsm-galleri-crd"].len(), 1);
        assert!(
            registry
                .by_issuer("https://other.example.org", None)
                .is_none()
        );
        assert!(
            registry
                .by_issuer("https://emr.example.org", Some("t"))
                .is_none()
        );
    }

    #[test]
    fn tenants_tell_apart_clients_that_share_an_issuer() {
        let registry = ClientRegistry::from_yaml(
            "audience: [a]\nclients:\n  - id: north\n    iss: https://cloud.example.org\n    tenant: north\n    jku: https://cloud.example.org/jwks\n  - id: south\n    iss: https://cloud.example.org\n    tenant: south\n    jku: https://cloud.example.org/jwks\n",
        )
        .unwrap();
        let iss = "https://cloud.example.org";
        assert_eq!(registry.by_issuer(iss, Some("north")).unwrap().id, "north");
        assert_eq!(registry.by_issuer(iss, Some("south")).unwrap().id, "south");
        assert!(registry.by_issuer(iss, Some("east")).is_none());
        assert!(registry.by_issuer(iss, None).is_none());

        let twice = "audience: [a]\nclients:\n  - id: x\n    iss: https://c\n    tenant: t\n    jku: https://c/jwks\n  - id: y\n    iss: https://c\n    tenant: t\n    jku: https://c/jwks\n";
        assert!(ClientRegistry::from_yaml(twice).is_err());
        let mixed = "audience: [a]\nclients:\n  - id: x\n    iss: https://c\n    jku: https://c/jwks\n  - id: y\n    iss: https://c\n    tenant: t\n    jku: https://c/jwks\n";
        assert!(ClientRegistry::from_yaml(mixed).is_err());
        let two_jku = "audience: [a]\nclients:\n  - id: x\n    iss: https://c\n    tenant: t\n    jku: https://c/jwks\n  - id: y\n    iss: https://c\n    tenant: u\n    jku: https://c/other\n";
        assert!(ClientRegistry::from_yaml(two_jku).is_err());
        let orphan = "clients:\n  - id: x\n    tenant: t\n";
        assert!(ClientRegistry::from_yaml(orphan).is_err());
    }

    #[test]
    fn refuses_duplicates_half_configured_signing_and_a_missing_audience() {
        let twice = "audience: [a]\nclients:\n  - id: x\n  - id: x\n";
        assert!(ClientRegistry::from_yaml(twice).is_err());
        let half = "audience: [a]\nclients:\n  - id: x\n    iss: https://e\n";
        assert!(ClientRegistry::from_yaml(half).is_err());
        let no_aud = "clients:\n  - id: x\n    iss: https://e\n    jku: https://e/jwks\n";
        assert!(ClientRegistry::from_yaml(no_aud).is_err());
        let bad_anon = "anonymous_client: nobody\nclients:\n  - id: x\n";
        assert!(ClientRegistry::from_yaml(bad_anon).is_err());
    }

    #[test]
    fn names_services_the_catalog_does_not_offer() {
        let registry =
            ClientRegistry::from_yaml("clients:\n  - id: x\n    services: [a, b]\n").unwrap();
        assert_eq!(
            registry.unknown_services(&["a".to_string()]),
            vec![("x", "b")]
        );
    }
}
