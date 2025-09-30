use crate::{SofBundle, SofError};
use async_trait::async_trait;
use helios_fhir::Element;
use object_store::{
    ObjectStore, aws::AmazonS3Builder, azure::MicrosoftAzureBuilder,
    gcp::GoogleCloudStorageBuilder, path::Path as ObjectPath,
};
use reqwest;
use serde_json;
use std::sync::Arc;
use tokio::fs;
use url::Url;

/// Trait for loading FHIR data from various sources
#[async_trait]
pub trait DataSource: Send + Sync {
    /// Load FHIR data from the source and return as a Bundle
    async fn load(&self, source: &str) -> Result<SofBundle, SofError>;
}

/// Implementation for loading data from various sources based on URL scheme
pub struct UniversalDataSource {
    client: reqwest::Client,
}

impl UniversalDataSource {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl Default for UniversalDataSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataSource for UniversalDataSource {
    async fn load(&self, source: &str) -> Result<SofBundle, SofError> {
        // Parse the source as a URL to determine the protocol
        let url = Url::parse(source).map_err(|e| {
            SofError::InvalidSource(format!("Invalid source URL '{}': {}", source, e))
        })?;

        match url.scheme() {
            "file" => load_from_file(&url).await,
            "http" | "https" => load_from_http(&self.client, &url).await,
            "s3" => load_from_s3(&url).await,
            "gs" => load_from_gcs(&url).await,
            "azure" | "abfss" | "abfs" => load_from_azure(&url).await,
            scheme => Err(SofError::UnsupportedSourceProtocol(format!(
                "Unsupported source protocol: {}. Supported: file://, http(s)://, s3://, gs://, azure://",
                scheme
            ))),
        }
    }
}

/// Load FHIR data from a local file
async fn load_from_file(url: &Url) -> Result<SofBundle, SofError> {
    // Convert file URL to path
    let path = url
        .to_file_path()
        .map_err(|_| SofError::InvalidSource(format!("Invalid file URL: {}", url)))?;

    // Check if file exists
    if !path.exists() {
        return Err(SofError::SourceNotFound(format!(
            "File not found: {}",
            path.display()
        )));
    }

    // Read file contents
    let contents = fs::read_to_string(&path)
        .await
        .map_err(|e| SofError::SourceReadError(format!("Failed to read file: {}", e)))?;

    // Parse and convert to bundle
    parse_fhir_content(&contents, &path.to_string_lossy())
}

/// Load FHIR data from HTTP/HTTPS URL
async fn load_from_http(client: &reqwest::Client, url: &Url) -> Result<SofBundle, SofError> {
    // Fetch content from URL
    let response = client
        .get(url.as_str())
        .header("Accept", "application/fhir+json, application/json")
        .send()
        .await
        .map_err(|e| {
            SofError::SourceFetchError(format!("Failed to fetch from URL '{}': {}", url, e))
        })?;

    // Check response status
    if !response.status().is_success() {
        return Err(SofError::SourceFetchError(format!(
            "HTTP error {} when fetching '{}'",
            response.status(),
            url
        )));
    }

    // Get content
    let contents = response
        .text()
        .await
        .map_err(|e| SofError::SourceReadError(format!("Failed to read response body: {}", e)))?;

    // Parse and convert to bundle
    parse_fhir_content(&contents, url.as_str())
}

/// Load FHIR data from AWS S3
async fn load_from_s3(url: &Url) -> Result<SofBundle, SofError> {
    // Parse S3 URL: s3://bucket/path/to/object
    let bucket = url.host_str().ok_or_else(|| {
        SofError::InvalidSource(format!("Invalid S3 URL '{}': missing bucket name", url))
    })?;

    let path = url.path().trim_start_matches('/');
    if path.is_empty() {
        return Err(SofError::InvalidSource(format!(
            "Invalid S3 URL '{}': missing object path",
            url
        )));
    }

    // Create S3 client using environment variables or default credentials
    let store = AmazonS3Builder::new()
        .with_bucket_name(bucket)
        .build()
        .map_err(|e| {
            SofError::SourceFetchError(format!(
                "Failed to create S3 client for '{}': {}. Ensure AWS credentials are configured (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_REGION)",
                url, e
            ))
        })?;

    load_from_object_store(Arc::new(store), path, url.as_str()).await
}

/// Load FHIR data from Google Cloud Storage
async fn load_from_gcs(url: &Url) -> Result<SofBundle, SofError> {
    // Parse GCS URL: gs://bucket/path/to/object
    let bucket = url.host_str().ok_or_else(|| {
        SofError::InvalidSource(format!("Invalid GCS URL '{}': missing bucket name", url))
    })?;

    let path = url.path().trim_start_matches('/');
    if path.is_empty() {
        return Err(SofError::InvalidSource(format!(
            "Invalid GCS URL '{}': missing object path",
            url
        )));
    }

    // Create GCS client using environment variables or default credentials
    let store = GoogleCloudStorageBuilder::new()
        .with_bucket_name(bucket)
        .build()
        .map_err(|e| {
            SofError::SourceFetchError(format!(
                "Failed to create GCS client for '{}': {}. Ensure GCP credentials are configured (GOOGLE_SERVICE_ACCOUNT or Application Default Credentials)",
                url, e
            ))
        })?;

    load_from_object_store(Arc::new(store), path, url.as_str()).await
}

/// Load FHIR data from Azure Blob Storage
async fn load_from_azure(url: &Url) -> Result<SofBundle, SofError> {
    // Parse Azure URL: azure://container/path/to/object or abfss://container@account.dfs.core.windows.net/path
    let (container, path) = if url.scheme() == "azure" {
        // Simple format: azure://container/path
        let container = url.host_str().ok_or_else(|| {
            SofError::InvalidSource(format!(
                "Invalid Azure URL '{}': missing container name",
                url
            ))
        })?;
        let path = url.path().trim_start_matches('/');
        (container.to_string(), path.to_string())
    } else {
        // ABFSS format: abfss://container@account.dfs.core.windows.net/path
        let host = url.host_str().ok_or_else(|| {
            SofError::InvalidSource(format!("Invalid Azure URL '{}': missing host", url))
        })?;
        let parts: Vec<&str> = host.split('@').collect();
        if parts.len() != 2 {
            return Err(SofError::InvalidSource(format!(
                "Invalid Azure URL '{}': expected format abfss://container@account.dfs.core.windows.net/path",
                url
            )));
        }
        let container = parts[0];
        let path = url.path().trim_start_matches('/');
        (container.to_string(), path.to_string())
    };

    if path.is_empty() {
        return Err(SofError::InvalidSource(format!(
            "Invalid Azure URL '{}': missing blob path",
            url
        )));
    }

    // Create Azure client using environment variables or managed identity
    let store = MicrosoftAzureBuilder::new()
        .with_container_name(&container)
        .build()
        .map_err(|e| {
            SofError::SourceFetchError(format!(
                "Failed to create Azure client for '{}': {}. Ensure Azure credentials are configured (AZURE_STORAGE_ACCOUNT and AZURE_STORAGE_ACCESS_KEY, or managed identity)",
                url, e
            ))
        })?;

    load_from_object_store(Arc::new(store), &path, url.as_str()).await
}

/// Common function to load from any object store
async fn load_from_object_store(
    store: Arc<dyn ObjectStore>,
    path: &str,
    source_name: &str,
) -> Result<SofBundle, SofError> {
    // Create object path
    let object_path = ObjectPath::from(path);

    // Download the object
    let result = store.get(&object_path).await.map_err(|e| match e {
        object_store::Error::NotFound { .. } => {
            SofError::SourceNotFound(format!("Object not found at '{}'", source_name))
        }
        _ => SofError::SourceFetchError(format!("Failed to fetch from '{}': {}", source_name, e)),
    })?;

    // Read the bytes
    let bytes = result
        .bytes()
        .await
        .map_err(|e| SofError::SourceReadError(format!("Failed to read object data: {}", e)))?;

    // Convert to string
    let contents = String::from_utf8(bytes.to_vec()).map_err(|e| {
        SofError::InvalidSourceContent(format!(
            "Content from '{}' is not valid UTF-8: {}",
            source_name, e
        ))
    })?;

    // Parse and convert to bundle
    parse_fhir_content(&contents, source_name)
}

/// Parse FHIR content and convert to SofBundle
fn parse_fhir_content(contents: &str, source_name: &str) -> Result<SofBundle, SofError> {
    // First, try to determine what type of content we have
    let value: serde_json::Value = serde_json::from_str(contents).map_err(|e| {
        SofError::InvalidSourceContent(format!(
            "Failed to parse JSON from '{}': {}",
            source_name, e
        ))
    })?;

    // Check if it's already a Bundle
    if let Some(resource_type) = value.get("resourceType").and_then(|v| v.as_str()) {
        if resource_type == "Bundle" {
            // Try parsing as each FHIR version
            #[cfg(feature = "R4")]
            if let Ok(bundle) = serde_json::from_value::<helios_fhir::r4::Bundle>(value.clone()) {
                return Ok(SofBundle::R4(bundle));
            }
            #[cfg(feature = "R4B")]
            if let Ok(bundle) = serde_json::from_value::<helios_fhir::r4b::Bundle>(value.clone()) {
                return Ok(SofBundle::R4B(bundle));
            }
            #[cfg(feature = "R5")]
            if let Ok(bundle) = serde_json::from_value::<helios_fhir::r5::Bundle>(value.clone()) {
                return Ok(SofBundle::R5(bundle));
            }
            #[cfg(feature = "R6")]
            if let Ok(bundle) = serde_json::from_value::<helios_fhir::r6::Bundle>(value.clone()) {
                return Ok(SofBundle::R6(bundle));
            }
            return Err(SofError::InvalidSourceContent(format!(
                "Bundle from '{}' could not be parsed as any supported FHIR version",
                source_name
            )));
        }

        // It's a single resource - wrap it in a Bundle
        return wrap_resource_in_bundle(value, source_name);
    }

    // Check if it's an array of resources
    if value.is_array() {
        return wrap_resources_in_bundle(value, source_name);
    }

    Err(SofError::InvalidSourceContent(format!(
        "Content from '{}' is not a valid FHIR resource or Bundle",
        source_name
    )))
}

/// Wrap a single resource in a Bundle
fn wrap_resource_in_bundle(
    resource: serde_json::Value,
    source_name: &str,
) -> Result<SofBundle, SofError> {
    // Try each FHIR version
    // R4
    #[cfg(feature = "R4")]
    if let Ok(res) = serde_json::from_value::<helios_fhir::r4::Resource>(resource.clone()) {
        let mut bundle = helios_fhir::r4::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        bundle.entry = Some(vec![helios_fhir::r4::BundleEntry {
            resource: Some(res),
            ..Default::default()
        }]);
        return Ok(SofBundle::R4(bundle));
    }

    // R4B
    #[cfg(feature = "R4B")]
    if let Ok(res) = serde_json::from_value::<helios_fhir::r4b::Resource>(resource.clone()) {
        let mut bundle = helios_fhir::r4b::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        bundle.entry = Some(vec![helios_fhir::r4b::BundleEntry {
            resource: Some(res),
            ..Default::default()
        }]);
        return Ok(SofBundle::R4B(bundle));
    }

    // R5
    #[cfg(feature = "R5")]
    if let Ok(res) = serde_json::from_value::<helios_fhir::r5::Resource>(resource.clone()) {
        let mut bundle = helios_fhir::r5::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        bundle.entry = Some(vec![helios_fhir::r5::BundleEntry {
            resource: Some(Box::new(res)),
            ..Default::default()
        }]);
        return Ok(SofBundle::R5(bundle));
    }

    // R6
    #[cfg(feature = "R6")]
    if let Ok(res) = serde_json::from_value::<helios_fhir::r6::Resource>(resource.clone()) {
        let mut bundle = helios_fhir::r6::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        bundle.entry = Some(vec![helios_fhir::r6::BundleEntry {
            resource: Some(Box::new(res)),
            ..Default::default()
        }]);
        return Ok(SofBundle::R6(bundle));
    }

    Err(SofError::InvalidSourceContent(format!(
        "Resource from '{}' could not be parsed as any supported FHIR version",
        source_name
    )))
}

/// Wrap an array of resources in a Bundle
fn wrap_resources_in_bundle(
    resources: serde_json::Value,
    source_name: &str,
) -> Result<SofBundle, SofError> {
    let arr = resources
        .as_array()
        .ok_or_else(|| SofError::InvalidSourceContent("Expected array of resources".to_string()))?;

    if arr.is_empty() {
        return Err(SofError::InvalidSourceContent(format!(
            "Empty array of resources from '{}'",
            source_name
        )));
    }

    // Try to parse the first resource to determine version
    let first = &arr[0];

    // Try R4
    #[cfg(feature = "R4")]
    if serde_json::from_value::<helios_fhir::r4::Resource>(first.clone()).is_ok() {
        let mut bundle = helios_fhir::r4::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        let mut entries = Vec::new();

        for resource in arr {
            let res = serde_json::from_value::<helios_fhir::r4::Resource>(resource.clone())
                .map_err(|e| {
                    SofError::InvalidSourceContent(format!(
                        "Failed to parse R4 resource from '{}': {}",
                        source_name, e
                    ))
                })?;
            entries.push(helios_fhir::r4::BundleEntry {
                resource: Some(res),
                ..Default::default()
            });
        }

        bundle.entry = Some(entries);
        return Ok(SofBundle::R4(bundle));
    }

    // Try R4B
    #[cfg(feature = "R4B")]
    if serde_json::from_value::<helios_fhir::r4b::Resource>(first.clone()).is_ok() {
        let mut bundle = helios_fhir::r4b::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        let mut entries = Vec::new();

        for resource in arr {
            let res = serde_json::from_value::<helios_fhir::r4b::Resource>(resource.clone())
                .map_err(|e| {
                    SofError::InvalidSourceContent(format!(
                        "Failed to parse R4B resource from '{}': {}",
                        source_name, e
                    ))
                })?;
            entries.push(helios_fhir::r4b::BundleEntry {
                resource: Some(res),
                ..Default::default()
            });
        }

        bundle.entry = Some(entries);
        return Ok(SofBundle::R4B(bundle));
    }

    // Try R5
    #[cfg(feature = "R5")]
    if serde_json::from_value::<helios_fhir::r5::Resource>(first.clone()).is_ok() {
        let mut bundle = helios_fhir::r5::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        let mut entries = Vec::new();

        for resource in arr {
            let res = serde_json::from_value::<helios_fhir::r5::Resource>(resource.clone())
                .map_err(|e| {
                    SofError::InvalidSourceContent(format!(
                        "Failed to parse R5 resource from '{}': {}",
                        source_name, e
                    ))
                })?;
            entries.push(helios_fhir::r5::BundleEntry {
                resource: Some(Box::new(res)),
                ..Default::default()
            });
        }

        bundle.entry = Some(entries);
        return Ok(SofBundle::R5(bundle));
    }

    // Try R6
    #[cfg(feature = "R6")]
    if serde_json::from_value::<helios_fhir::r6::Resource>(first.clone()).is_ok() {
        let mut bundle = helios_fhir::r6::Bundle::default();
        bundle.r#type = Element {
            id: None,
            extension: None,
            value: Some("collection".to_string()),
        };
        let mut entries = Vec::new();

        for resource in arr {
            let res = serde_json::from_value::<helios_fhir::r6::Resource>(resource.clone())
                .map_err(|e| {
                    SofError::InvalidSourceContent(format!(
                        "Failed to parse R6 resource from '{}': {}",
                        source_name, e
                    ))
                })?;
            entries.push(helios_fhir::r6::BundleEntry {
                resource: Some(Box::new(res)),
                ..Default::default()
            });
        }

        bundle.entry = Some(entries);
        return Ok(SofBundle::R6(bundle));
    }

    Err(SofError::InvalidSourceContent(format!(
        "Resources from '{}' could not be parsed as any supported FHIR version",
        source_name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_fhir_bundle() {
        let bundle_json = r#"{
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [{
                "resource": {
                    "resourceType": "Patient",
                    "id": "123"
                }
            }]
        }"#;

        let result = parse_fhir_content(bundle_json, "test").unwrap();
        #[cfg(feature = "R4")]
        assert!(matches!(result, SofBundle::R4(_)));
        #[cfg(not(feature = "R4"))]
        assert!(matches!(result, _));
    }

    #[tokio::test]
    async fn test_parse_single_resource() {
        let patient_json = r#"{
            "resourceType": "Patient",
            "id": "123"
        }"#;

        let result = parse_fhir_content(patient_json, "test").unwrap();
        #[cfg(feature = "R4")]
        match result {
            SofBundle::R4(bundle) => {
                assert_eq!(bundle.entry.as_ref().unwrap().len(), 1);
            }
            #[cfg(feature = "R4B")]
            SofBundle::R4B(_) => panic!("Expected R4 bundle"),
            #[cfg(feature = "R5")]
            SofBundle::R5(_) => panic!("Expected R4 bundle"),
            #[cfg(feature = "R6")]
            SofBundle::R6(_) => panic!("Expected R4 bundle"),
        }
    }

    #[tokio::test]
    async fn test_parse_resource_array() {
        let resources_json = r#"[
            {
                "resourceType": "Patient",
                "id": "123"
            },
            {
                "resourceType": "Patient",
                "id": "456"
            }
        ]"#;

        let result = parse_fhir_content(resources_json, "test").unwrap();
        #[cfg(feature = "R4")]
        match result {
            SofBundle::R4(bundle) => {
                assert_eq!(bundle.entry.as_ref().unwrap().len(), 2);
            }
            #[cfg(feature = "R4B")]
            SofBundle::R4B(_) => panic!("Expected R4 bundle"),
            #[cfg(feature = "R5")]
            SofBundle::R5(_) => panic!("Expected R4 bundle"),
            #[cfg(feature = "R6")]
            SofBundle::R6(_) => panic!("Expected R4 bundle"),
        }
    }

    #[tokio::test]
    async fn test_invalid_content() {
        let invalid_json = r#"{"not": "fhir"}"#;
        let result = parse_fhir_content(invalid_json, "test");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_s3_url_parsing() {
        let data_source = UniversalDataSource::new();

        // Test invalid S3 URL without bucket
        let result = data_source.load("s3:///path/to/file.json").await;
        assert!(result.is_err());
        if let Err(SofError::InvalidSource(msg)) = result {
            assert!(msg.contains("missing bucket name"));
        }

        // Test invalid S3 URL without path
        let result = data_source.load("s3://bucket/").await;
        assert!(result.is_err());
        if let Err(SofError::InvalidSource(msg)) = result {
            assert!(msg.contains("missing object path"));
        }

        // Note: Actual S3 fetching would require valid credentials and a real bucket
        // These tests verify URL parsing and error handling
    }

    #[tokio::test]
    async fn test_gcs_url_parsing() {
        let data_source = UniversalDataSource::new();

        // Test invalid GCS URL without bucket
        let result = data_source.load("gs:///path/to/file.json").await;
        assert!(result.is_err());
        if let Err(SofError::InvalidSource(msg)) = result {
            assert!(msg.contains("missing bucket name"));
        }

        // Test invalid GCS URL without path
        let result = data_source.load("gs://bucket/").await;
        assert!(result.is_err());
        if let Err(SofError::InvalidSource(msg)) = result {
            assert!(msg.contains("missing object path"));
        }
    }

    #[tokio::test]
    async fn test_azure_url_parsing() {
        let data_source = UniversalDataSource::new();

        // Test invalid Azure URL without container
        let result = data_source.load("azure:///path/to/file.json").await;
        assert!(result.is_err());
        if let Err(SofError::InvalidSource(msg)) = result {
            assert!(msg.contains("missing container name"));
        }

        // Test invalid Azure URL without path
        let result = data_source.load("azure://container/").await;
        assert!(result.is_err());
        if let Err(SofError::InvalidSource(msg)) = result {
            assert!(msg.contains("missing blob path"));
        }
    }

    #[tokio::test]
    async fn test_unsupported_protocol() {
        let data_source = UniversalDataSource::new();

        // Test unsupported protocol
        let result = data_source.load("ftp://server/file.json").await;
        assert!(result.is_err());
        if let Err(SofError::UnsupportedSourceProtocol(msg)) = result {
            assert!(msg.contains("Unsupported source protocol: ftp"));
            assert!(msg.contains("Supported:"));
        }
    }
}
