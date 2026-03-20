//! S3 API abstraction — trait definition, request/response types, AWS SDK
//! client implementation, and SDK error mapping.

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_sdk_s3::Client;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use chrono::{DateTime, Utc};

/// Metadata returned from S3 object head and put operations.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ObjectMetadata {
    /// ETag returned by S3; used as an optimistic concurrency token for
    /// conditional writes.
    pub etag: Option<String>,
    /// Wall-clock time of the last write, if returned by the operation.
    pub last_modified: Option<DateTime<Utc>>,
    /// Object size in bytes.
    pub size: i64,
}

/// Full S3 object body together with its metadata.
#[derive(Debug, Clone)]
pub struct ObjectData {
    /// Raw object bytes.
    pub bytes: Vec<u8>,
    /// Metadata associated with the object at the time it was fetched.
    pub metadata: ObjectMetadata,
}

/// A single entry returned by a `ListObjects` call.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ListObjectItem {
    /// Full S3 object key.
    pub key: String,
    /// ETag of the object at the time of listing.
    pub etag: Option<String>,
    /// Last-modified timestamp of the object.
    pub last_modified: Option<DateTime<Utc>>,
    /// Object size in bytes.
    pub size: i64,
}

/// Paginated result set from a `ListObjects` call.
#[derive(Debug, Clone)]
pub struct ListObjectsResult {
    /// Objects matching the requested prefix in this page.
    pub items: Vec<ListObjectItem>,
    /// Continuation token to retrieve the next page, or `None` if this is the
    /// last page.
    pub next_continuation_token: Option<String>,
}

/// Normalised error variants returned by the S3 API abstraction.
///
/// These are mapped from SDK-specific errors so that callers do not need to
/// depend on the AWS SDK error types directly.
#[derive(Debug, Clone)]
pub enum S3ClientError {
    /// The requested bucket or object does not exist.
    NotFound,
    /// A conditional write failed because the ETag or existence precondition
    /// was not satisfied (`If-Match` or `If-None-Match: *`).
    PreconditionFailed,
    /// The request was rate-limited by S3.
    Throttled(String),
    /// The service was unreachable (timeout, dispatch failure, etc.).
    Unavailable(String),
    /// The request was rejected due to invalid input (bad bucket name, etc.).
    InvalidInput(String),
    /// An unexpected error occurred inside the SDK or service.
    Internal(String),
}

/// Abstraction over the AWS S3 API surface used by this backend.
///
/// Implemented by `AwsS3Client` in production and by a `MockS3Client` in
/// tests, allowing the backend logic to be exercised without a real AWS
/// account.
#[async_trait]
pub trait S3Api: Send + Sync {
    /// Checks that `bucket` exists and is accessible to the current
    /// credentials.
    async fn head_bucket(&self, bucket: &str) -> Result<(), S3ClientError>;

    /// Returns object metadata if the key exists, or `None` if not found.
    async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectMetadata>, S3ClientError>;

    /// Downloads the full object body, returning `None` if the key does not
    /// exist.
    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectData>, S3ClientError>;

    /// Uploads `body` to the given key.
    ///
    /// `if_match` enforces that the existing ETag matches before overwriting.
    /// `if_none_match = Some("*")` prevents overwriting an existing object.
    /// Both conditions return `PreconditionFailed` on mismatch.
    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ObjectMetadata, S3ClientError>;

    /// Deletes the object at the given key. Succeeds even if the key does not
    /// exist.
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), S3ClientError>;

    /// Lists objects whose keys start with `prefix`, with optional
    /// cursor-based pagination via `continuation`.
    async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: Option<i32>,
    ) -> Result<ListObjectsResult, S3ClientError>;
}

/// Production `S3Api` implementation backed by the AWS SDK.
#[derive(Debug, Clone)]
pub struct AwsS3Client {
    /// Underlying AWS SDK S3 client.
    client: Client,
}

#[derive(Debug, Clone, Default)]
pub struct AwsS3ClientOptions {
    pub endpoint_url: Option<String>,
    pub force_path_style: bool,
}

impl AwsS3Client {
    /// Constructs a client from a pre-loaded AWS SDK configuration.
    #[allow(dead_code)]
    pub fn from_sdk_config(config: &SdkConfig) -> Self {
        Self::from_sdk_config_with_options(config, AwsS3ClientOptions::default())
    }

    /// Constructs a client from a pre-loaded AWS SDK configuration with S3-compatible overrides.
    ///
    /// This is used for non-AWS S3 endpoints (e.g., MinIO, local S3-compatible gateways) where
    /// callers may need to:
    /// - override the endpoint URL (`options.endpoint_url`)
    /// - force path-style addressing (`options.force_path_style`)
    ///
    /// When `endpoint_url` is `None`, the SDK will use the default AWS endpoint resolution
    /// derived from `config` (region, partitions, etc.).
    pub fn from_sdk_config_with_options(config: &SdkConfig, options: AwsS3ClientOptions) -> Self {
        let mut builder = aws_sdk_s3::config::Builder::from(config);

        // Override endpoint for S3-compatible providers (e.g., MinIO).
        if let Some(endpoint_url) = options.endpoint_url {
            builder = builder.endpoint_url(endpoint_url);
        }

        // Some S3-compatible providers require path-style requests (bucket in path vs subdomain).
        builder = builder.force_path_style(options.force_path_style);

        let s3_config = builder.build();
        Self {
            client: Client::from_conf(s3_config),
        }
    }

    /// Loads the AWS SDK configuration from the environment.
    ///
    /// If `region` is `Some`, it overrides the region from the environment;
    /// otherwise the standard provider chain is used (shared config file,
    /// environment variables, EC2 instance metadata, etc.).
    pub async fn load_sdk_config(region: Option<&str>) -> SdkConfig {
        let mut loader = aws_config::defaults(BehaviorVersion::latest());
        if let Some(region) = region {
            loader = loader.region(Region::new(region.to_string()));
        }
        loader.load().await
    }
}

#[async_trait]
impl S3Api for AwsS3Client {
    async fn head_bucket(&self, bucket: &str) -> Result<(), S3ClientError> {
        self.client
            .head_bucket()
            .bucket(bucket)
            .send()
            .await
            .map_err(map_sdk_error)?;
        Ok(())
    }

    async fn head_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectMetadata>, S3ClientError> {
        match self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => Ok(Some(ObjectMetadata {
                etag: out.e_tag().map(|s| s.to_string()),
                last_modified: None,
                size: out.content_length().unwrap_or_default(),
            })),
            Err(err) => {
                let mapped = map_sdk_error(err);
                if matches!(mapped, S3ClientError::NotFound) {
                    Ok(None)
                } else {
                    Err(mapped)
                }
            }
        }
    }

    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectData>, S3ClientError> {
        match self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => {
                let etag = out.e_tag().map(|s| s.to_string());
                let bytes = out
                    .body
                    .collect()
                    .await
                    .map_err(|e| {
                        S3ClientError::Internal(format!("failed to collect object body: {e}"))
                    })?
                    .into_bytes()
                    .to_vec();
                Ok(Some(ObjectData {
                    metadata: ObjectMetadata {
                        etag,
                        last_modified: None,
                        size: bytes.len() as i64,
                    },
                    bytes,
                }))
            }
            Err(err) => {
                let mapped = map_sdk_error(err);
                if matches!(mapped, S3ClientError::NotFound) {
                    Ok(None)
                } else {
                    Err(mapped)
                }
            }
        }
    }

    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
        if_match: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<ObjectMetadata, S3ClientError> {
        let mut req = self
            .client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(ByteStream::from(body));

        if let Some(content_type) = content_type {
            req = req.content_type(content_type);
        }
        if let Some(if_match) = if_match {
            req = req.if_match(if_match);
        }
        if let Some(if_none_match) = if_none_match {
            req = req.if_none_match(if_none_match);
        }

        let out = req.send().await.map_err(map_sdk_error)?;

        Ok(ObjectMetadata {
            etag: out.e_tag().map(|s| s.to_string()),
            last_modified: None,
            size: 0,
        })
    }

    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), S3ClientError> {
        self.client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(map_sdk_error)?;
        Ok(())
    }

    async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
        continuation: Option<&str>,
        max_keys: Option<i32>,
    ) -> Result<ListObjectsResult, S3ClientError> {
        let mut req = self.client.list_objects_v2().bucket(bucket).prefix(prefix);

        if let Some(token) = continuation {
            req = req.continuation_token(token);
        }
        if let Some(max_keys) = max_keys {
            req = req.max_keys(max_keys);
        }

        let out = req.send().await.map_err(map_sdk_error)?;
        let mut items = Vec::new();

        for item in out.contents() {
            if let Some(key) = item.key() {
                items.push(ListObjectItem {
                    key: key.to_string(),
                    etag: item.e_tag().map(|s| s.to_string()),
                    last_modified: None,
                    size: item.size().unwrap_or_default(),
                });
            }
        }

        Ok(ListObjectsResult {
            items,
            next_continuation_token: out.next_continuation_token().map(|s| s.to_string()),
        })
    }
}

/// Maps an AWS SDK error to the normalised `S3ClientError` taxonomy.
///
/// Known service error codes are matched to specific variants; everything
/// else falls through to `Internal`.
fn map_sdk_error<E>(err: aws_sdk_s3::error::SdkError<E>) -> S3ClientError
where
    E: ProvideErrorMetadata + std::fmt::Debug,
{
    match err {
        aws_sdk_s3::error::SdkError::ServiceError(service_err) => {
            let code = service_err.err().code().unwrap_or("Unknown");
            let message = service_err
                .err()
                .message()
                .map(str::to_string)
                .unwrap_or_default();
            match code {
                "NoSuchKey" | "NotFound" | "NoSuchBucket" => S3ClientError::NotFound,
                "PreconditionFailed" => S3ClientError::PreconditionFailed,
                "SlowDown" | "Throttling" | "ThrottlingException" => {
                    S3ClientError::Throttled(message)
                }
                "InvalidBucketName" | "InvalidArgument" => S3ClientError::InvalidInput(message),
                "AccessDenied"
                | "InvalidAccessKeyId"
                | "SignatureDoesNotMatch"
                | "ExpiredToken" => S3ClientError::Unavailable(format!("access denied: {code}")),
                _ => {
                    // When S3 returns no error code (e.g. HeadBucket 403),
                    // fall back to the HTTP status for a cleaner message.
                    let status = service_err.raw().status().as_u16();
                    match status {
                        403 => S3ClientError::Unavailable(
                            "access denied (HTTP 403) — check AWS credentials and bucket policy"
                                .to_string(),
                        ),
                        404 => S3ClientError::NotFound,
                        _ => {
                            let detail = if message.is_empty() {
                                format!("{:?}", service_err.err())
                            } else {
                                message
                            };
                            S3ClientError::Internal(format!(
                                "S3 error (HTTP {status}, code={code}): {detail}"
                            ))
                        }
                    }
                }
            }
        }
        aws_sdk_s3::error::SdkError::TimeoutError(_) => {
            S3ClientError::Unavailable("request timed out".to_string())
        }
        aws_sdk_s3::error::SdkError::DispatchFailure(err) => {
            S3ClientError::Unavailable(format!("connection failed: {err:?}"))
        }
        _ => S3ClientError::Internal(format!("{err}")),
    }
}
