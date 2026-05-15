use serde::Serialize;
use serde_json::Value;

use crate::config::AuthConfig;

/// SMART on FHIR configuration document served at
/// `/.well-known/smart-configuration`.
#[derive(Debug, Serialize)]
pub struct SmartConfiguration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwks_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub introspection_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub token_endpoint_auth_signing_alg_values_supported: Vec<String>,
    pub capabilities: Vec<String>,
}

impl SmartConfiguration {
    /// Build the SMART configuration document from `AuthConfig`.
    pub fn from_config(config: &AuthConfig) -> Self {
        Self {
            issuer: config.expected_issuer.clone(),
            jwks_uri: config
                .smart_jwks_url
                .clone()
                .or_else(|| config.jwks_url.clone()),
            authorization_endpoint: config.smart_authorize_endpoint.clone(),
            token_endpoint: config.smart_token_endpoint.clone(),
            introspection_endpoint: config.smart_introspection_endpoint.clone(),
            management_endpoint: config.smart_management_endpoint.clone(),
            registration_endpoint: config.smart_registration_endpoint.clone(),
            revocation_endpoint: config.smart_revocation_endpoint.clone(),
            scopes_supported: vec![
                "system/*.cruds".to_string(),
                "system/*.rs".to_string(),
                "system/*.r".to_string(),
            ],
            response_types_supported: vec!["token".to_string()],
            grant_types_supported: vec!["client_credentials".to_string()],
            token_endpoint_auth_methods_supported: vec!["private_key_jwt".to_string()],
            token_endpoint_auth_signing_alg_values_supported: vec![
                "RS384".to_string(),
                "ES384".to_string(),
            ],
            capabilities: vec![
                "permission-v2".to_string(),
                "client-confidential-asymmetric".to_string(),
            ],
        }
    }

    /// Serialize to a JSON `Value`.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("SmartConfiguration serialization cannot fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smart_config_from_default() {
        let config = AuthConfig::default();
        let smart = SmartConfiguration::from_config(&config);

        assert!(smart.issuer.is_none());
        assert!(smart.token_endpoint.is_none());
        assert!(smart.capabilities.contains(&"permission-v2".to_string()));
    }

    #[test]
    fn test_smart_config_with_values() {
        let config = AuthConfig {
            expected_issuer: Some("https://idp.example.com".to_string()),
            smart_token_endpoint: Some("https://idp.example.com/token".to_string()),
            smart_jwks_url: Some("https://idp.example.com/.well-known/jwks.json".to_string()),
            ..AuthConfig::default()
        };
        let smart = SmartConfiguration::from_config(&config);

        assert_eq!(smart.issuer.as_deref(), Some("https://idp.example.com"));
        assert_eq!(
            smart.token_endpoint.as_deref(),
            Some("https://idp.example.com/token")
        );
        assert_eq!(
            smart.jwks_uri.as_deref(),
            Some("https://idp.example.com/.well-known/jwks.json")
        );
    }

    #[test]
    fn test_smart_config_json_serialization() {
        let config = AuthConfig {
            expected_issuer: Some("https://idp.example.com".to_string()),
            ..AuthConfig::default()
        };
        let smart = SmartConfiguration::from_config(&config);
        let json = smart.to_json();

        assert!(json["capabilities"].is_array());
        assert!(json["scopes_supported"].is_array());
        // Fields that are None should be omitted
        assert!(json.get("authorization_endpoint").is_none());
    }
}
