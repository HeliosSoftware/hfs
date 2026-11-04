//! Retry policy for S3 `GetObject` calls whose body read fails (#2048).
//!
//! The AWS SDK retries a request only until the response headers arrive.
//! `GetObject` hands the body back unread, so a failure while the body is
//! being read (a stalled-stream trip, a mid-body connection reset, a truncated
//! body, a checksum mismatch) is never retried by the SDK. A GET is
//! idempotent, so this module re-issues the whole request (send plus collect)
//! instead.
//!
//! The policy lives here, apart from [`super::client::AwsS3Client`], so it can
//! be unit-tested without the SDK: [`get_with_body_retry`] takes the attempt as
//! a closure.

use std::error::Error;
use std::future::Future;
use std::time::Duration;

use aws_sdk_s3::error::DisplayErrorContext;
use tracing::warn;

use super::client::{ObjectData, S3ClientError};

/// How often, and how patiently, a `GetObject` whose body read failed is
/// re-issued.
#[derive(Debug, Clone, Copy)]
pub(super) struct GetRetryPolicy {
    /// Attempts in all, the first included.
    pub max_attempts: u32,
    /// The backoff before the second attempt; it doubles for each one after.
    pub base: Duration,
    /// The longest backoff.
    pub cap: Duration,
}

impl GetRetryPolicy {
    /// The backoff after `attempt` (1-based) attempts have failed: `base`
    /// doubled for each earlier failure, and never more than `cap`.
    fn backoff(&self, attempt: u32) -> Duration {
        self.base
            .saturating_mul(1u32 << attempt.saturating_sub(1).min(16))
            .min(self.cap)
    }
}

/// The policy for `GetObject`: 3 attempts in all, a 100 ms base backoff and a
/// 1.6 s cap.
///
/// The pause after the first failure is drawn from `[0, 100 ms]` and the one
/// after the second from `[0, 200 ms]`; the cap only matters if the attempts
/// are raised. Three is enough to ride out one dropped connection, and few
/// enough that a real outage fails fast rather than tripling the load (a scan
/// reads up to 32 objects at once).
pub(super) const GET_OBJECT_RETRY: GetRetryPolicy = GetRetryPolicy {
    max_attempts: 3,
    base: Duration::from_millis(100),
    cap: Duration::from_millis(1600),
};

/// A uniform draw from `[0, 1)`, for jitter.
///
/// Taken from a v4 UUID, which the crate already depends on, rather than from a
/// new RNG dependency: the first 48 bits of a v4 UUID are all random (the
/// version and variant bits sit after them), and 48 bits fit a `f64` mantissa
/// exactly.
///
/// The same as `backends::mongodb::retry::jitter_fraction`, which is gated on
/// the `mongodb` feature and so cannot be shared with this backend.
pub(super) fn jitter_fraction() -> f64 {
    let bits = (uuid::Uuid::new_v4().as_u128() >> 80) as u64;
    bits as f64 / (1u64 << 48) as f64
}

/// Full jitter: `fraction` of `backoff`, so the pause is anywhere from none at
/// all to the whole backoff. An out-of-range `fraction` is clamped rather than
/// trusted.
pub(super) fn full_jitter(backoff: Duration, fraction: f64) -> Duration {
    // `NaN` compares false to everything, so it must be caught before `clamp`.
    let fraction = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    backoff.mul_f64(fraction)
}

/// Why one `GetObject` attempt failed.
#[derive(Debug)]
pub(super) enum GetAttemptError {
    /// The request itself failed. Already classified, and never retried here:
    /// the SDK has retried the request phase, and `NotFound`,
    /// `BucketNotFound`, `PreconditionFailed`, `InvalidInput`, 403 and the
    /// other 4xx answers are not transient.
    Request(S3ClientError),
    /// The response headers arrived, but reading the body failed.
    Body {
        /// The error and its sources, `Display`ed and joined with ": ". Safe to
        /// show a client: it carries no object key and no `Debug` tail.
        chain: String,
        /// The same failure rendered with `DisplayErrorContext`, for the log.
        detail: String,
    },
}

impl GetAttemptError {
    /// A body-read failure.
    ///
    /// The SDK wraps every one of them as "streaming error", so the log needs
    /// the full chain to tell a stall from a reset, a truncation or a checksum
    /// mismatch.
    pub(super) fn body(err: &(dyn Error + 'static)) -> Self {
        Self::Body {
            chain: error_chain(err),
            detail: DisplayErrorContext(err).to_string(),
        }
    }
}

/// The `Display` of `err` and of each of its sources, joined with ": ".
fn error_chain(err: &(dyn Error + 'static)) -> String {
    let mut chain = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
        source = cause.source();
    }
    chain
}

/// Runs `attempt`, re-running it when the failure was in reading the object
/// body.
///
/// - A success is returned as it is, `Ok(None)` (the object is absent)
///   included.
/// - A [`GetAttemptError::Request`] is returned at once, unretried.
/// - A [`GetAttemptError::Body`] pauses for a full-jitter backoff and then runs
///   the whole attempt again, up to `policy.max_attempts` in all. When they run
///   out, the result is [`S3ClientError::Unavailable`] naming the cause.
///
/// Every failed body read is logged at `warn` with `bucket`, `key` and the full
/// error chain, so the log shows a stall, a reset, a truncation or a checksum
/// mismatch even when a retry then succeeds.
///
/// The `Unavailable` message is shown to clients verbatim, so it carries the
/// source chain only: not the object key, and not the `Debug` tail.
pub(super) async fn get_with_body_retry<F, Fut>(
    policy: &GetRetryPolicy,
    bucket: &str,
    key: &str,
    mut attempt: F,
) -> Result<Option<ObjectData>, S3ClientError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<ObjectData>, GetAttemptError>>,
{
    let max = policy.max_attempts.max(1);
    let mut made = 0u32;
    loop {
        made += 1;
        match attempt().await {
            Ok(found) => return Ok(found),
            Err(GetAttemptError::Request(err)) => return Err(err),
            Err(GetAttemptError::Body { chain, detail }) => {
                if made >= max {
                    warn!(
                        bucket,
                        key,
                        attempts = made,
                        error = %detail,
                        "S3 GetObject body read failed on every attempt; giving up"
                    );
                    return Err(S3ClientError::Unavailable(format!(
                        "failed to read object body after {made} attempts: {chain}"
                    )));
                }
                let delay = full_jitter(policy.backoff(made), jitter_fraction());
                warn!(
                    bucket,
                    key,
                    attempt = made,
                    max_attempts = max,
                    delay_ms = delay.as_millis() as u64,
                    error = %detail,
                    "S3 GetObject body read failed; re-issuing the request"
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::io;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use aws_sdk_s3::primitives::ByteStreamError;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::super::client::{AwsS3Client, AwsS3ClientOptions, ObjectMetadata, S3Api};
    use super::*;

    fn reset_error() -> GetAttemptError {
        GetAttemptError::body(&ByteStreamError::from(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        )))
    }

    fn object(bytes: &[u8], etag: &str) -> ObjectData {
        ObjectData {
            bytes: bytes.to_vec(),
            metadata: ObjectMetadata {
                etag: Some(etag.to_string()),
                last_modified: None,
                size: bytes.len() as i64,
            },
        }
    }

    /// Runs `get_with_body_retry` over `outcomes`, one per attempt in order, and
    /// returns the result with the number of attempts made. An attempt past the
    /// end of `outcomes` panics: the retry must not make more than the test
    /// scripted.
    async fn run_scripted(
        outcomes: Vec<Result<Option<ObjectData>, GetAttemptError>>,
    ) -> (Result<Option<ObjectData>, S3ClientError>, usize) {
        let calls = Cell::new(0usize);
        let script = RefCell::new(VecDeque::from(outcomes));
        let result = get_with_body_retry(&GET_OBJECT_RETRY, "bucket", "key", || {
            calls.set(calls.get() + 1);
            let next = script
                .borrow_mut()
                .pop_front()
                .expect("the retry made more attempts than the test scripted");
            async move { next }
        })
        .await;
        (result, calls.get())
    }

    #[test]
    fn backoff_doubles_from_the_base_and_stops_at_the_cap() {
        assert_eq!(GET_OBJECT_RETRY.max_attempts, 3);
        assert_eq!(GET_OBJECT_RETRY.backoff(1), Duration::from_millis(100));
        assert_eq!(GET_OBJECT_RETRY.backoff(2), Duration::from_millis(200));
        assert_eq!(GET_OBJECT_RETRY.backoff(5), Duration::from_millis(1600));
        assert_eq!(GET_OBJECT_RETRY.backoff(30), Duration::from_millis(1600));
    }

    #[test]
    fn full_jitter_stays_between_zero_and_the_backoff() {
        let backoff = Duration::from_millis(200);
        assert_eq!(full_jitter(backoff, 0.0), Duration::ZERO);
        assert_eq!(full_jitter(backoff, 0.5), Duration::from_millis(100));
        assert_eq!(full_jitter(backoff, 1.0), backoff);
        assert_eq!(full_jitter(backoff, 7.0), backoff);
        assert_eq!(full_jitter(backoff, -1.0), Duration::ZERO);
        assert_eq!(full_jitter(backoff, f64::NAN), Duration::ZERO);
    }

    #[test]
    fn jitter_fraction_is_in_the_unit_interval_and_varies() {
        let draws: Vec<f64> = (0..256).map(|_| jitter_fraction()).collect();
        assert!(draws.iter().all(|f| (0.0..1.0).contains(f)));
        assert!(
            draws.iter().any(|f| *f != draws[0]),
            "256 draws were all equal"
        );
    }

    /// The acceptance test: a body that fails once. The retried attempt's bytes
    /// and ETag come back together.
    #[tokio::test(start_paused = true)]
    async fn a_body_failure_is_retried_and_the_retried_attempt_is_returned() {
        let (result, calls) = run_scripted(vec![
            Err(reset_error()),
            Ok(Some(object(b"{\"id\":\"p1\"}", "\"e2\""))),
        ])
        .await;
        let data = result
            .expect("the retried attempt succeeded")
            .expect("found");
        assert_eq!(data.bytes, b"{\"id\":\"p1\"}");
        assert_eq!(data.metadata.etag.as_deref(), Some("\"e2\""));
        assert_eq!(calls, 2);
    }

    #[tokio::test(start_paused = true)]
    #[tracing_test::traced_test]
    async fn body_failures_stop_after_three_attempts_as_unavailable() {
        let (result, calls) = run_scripted(vec![
            Err(reset_error()),
            Err(reset_error()),
            Err(reset_error()),
        ])
        .await;
        match result {
            Err(S3ClientError::Unavailable(message)) => {
                assert!(
                    message.starts_with("failed to read object body after 3 attempts"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("connection reset by peer"),
                    "unexpected message: {message}"
                );
                assert!(
                    !message.contains("key"),
                    "the message must not carry the object key: {message}"
                );
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert_eq!(calls, 3);
        assert!(logs_contain("re-issuing the request"));
        assert!(logs_contain("giving up"));
        assert!(logs_contain("connection reset by peer"));
    }

    #[tokio::test(start_paused = true)]
    async fn request_errors_are_never_retried() {
        let errors = [
            S3ClientError::PreconditionFailed,
            S3ClientError::InvalidInput("bad".to_string()),
            S3ClientError::BucketNotFound("bucket".to_string()),
            S3ClientError::Throttled("slow down".to_string()),
            S3ClientError::Unavailable("connection failed".to_string()),
            S3ClientError::Internal("boom".to_string()),
        ];
        for error in errors {
            let started = tokio::time::Instant::now();
            let (result, calls) =
                run_scripted(vec![Err(GetAttemptError::Request(error.clone()))]).await;
            let got = result.expect_err("a request error is returned");
            assert_eq!(
                std::mem::discriminant(&got),
                std::mem::discriminant(&error),
                "{error:?} came back as {got:?}"
            );
            assert_eq!(calls, 1, "{error:?} was retried");
            assert_eq!(started.elapsed(), Duration::ZERO, "{error:?} paused");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_absent_object_is_not_retried() {
        let (result, calls) = run_scripted(vec![Ok(None)]).await;
        assert!(matches!(result, Ok(None)));
        assert_eq!(calls, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_error_on_a_reissued_attempt_ends_the_retry() {
        let (result, calls) = run_scripted(vec![
            Err(reset_error()),
            Err(GetAttemptError::Request(S3ClientError::Unavailable(
                "connection failed".to_string(),
            ))),
        ])
        .await;
        match result {
            Err(S3ClientError::Unavailable(message)) => assert_eq!(message, "connection failed"),
            other => panic!("expected the request error, got {other:?}"),
        }
        assert_eq!(calls, 2);

        // The object was deleted between the attempts.
        let (result, calls) = run_scripted(vec![Err(reset_error()), Ok(None)]).await;
        assert!(matches!(result, Ok(None)));
        assert_eq!(calls, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_pauses_are_bounded_by_the_policy() {
        let started = tokio::time::Instant::now();
        let (result, calls) = run_scripted(vec![
            Err(reset_error()),
            Err(reset_error()),
            Err(reset_error()),
        ])
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 3);
        // Two pauses, drawn from [0, 100 ms] and [0, 200 ms]. The timer rounds
        // each sleep up to a whole millisecond, so allow for that.
        let longest = GET_OBJECT_RETRY.backoff(1) + GET_OBJECT_RETRY.backoff(2);
        assert!(
            started.elapsed() <= longest + Duration::from_millis(2),
            "paused {:?}, more than the policy's {longest:?}",
            started.elapsed()
        );
    }

    #[test]
    fn the_error_chain_names_the_underlying_cause() {
        let reset = GetAttemptError::body(&ByteStreamError::from(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        )));
        match reset {
            GetAttemptError::Body { chain, detail } => {
                assert_eq!(chain, "IO error: connection reset by peer");
                assert!(detail.contains(&chain), "detail: {detail}");
                assert!(detail.contains("ConnectionReset"), "detail: {detail}");
            }
            other => panic!("expected a body error, got {other:?}"),
        }

        #[derive(Debug)]
        struct Outer(Inner);
        #[derive(Debug)]
        struct Inner;
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "outer")
            }
        }
        impl std::fmt::Display for Inner {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "inner")
            }
        }
        impl Error for Outer {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&self.0)
            }
        }
        impl Error for Inner {}
        assert_eq!(error_chain(&Outer(Inner)), "outer: inner");
    }

    // ------------------------------------------------------------------
    // The real `AwsS3Client` and SDK against an in-process 127.0.0.1 stub:
    // no Docker, no AWS. These prove the SDK does not retry the body phase
    // itself and that the wiring in `AwsS3Client::get_object` re-issues it.
    // ------------------------------------------------------------------

    const LOOPBACK_KEY: &str = "Patient/p1/current.json";
    const LOOPBACK_BODY: &[u8] = b"{\"resourceType\":\"Patient\",\"id\":\"p1\",\"active\":true}";

    /// A raw HTTP/1.1 200 whose `Content-Length` promises the whole of `body`
    /// but which carries only its first `sent` bytes (all of them when `sent`
    /// is `None`).
    fn raw_response(etag: &str, body: &[u8], sent: Option<usize>) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"{etag}\"\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(&body[..sent.unwrap_or(body.len())]);
        response
    }

    fn raw_not_found() -> Vec<u8> {
        let body = b"<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>";
        let mut response = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    /// Serves `script` in order, one raw response per request; a request past
    /// the end of the script gets the last response again. Returns the address
    /// and the count of requests served.
    async fn spawn_stub(script: Vec<Vec<u8>>) -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let served = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&served);
        let script = Arc::new(script);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let counter = Arc::clone(&counter);
                let script = Arc::clone(&script);
                // One task per connection, so an idle connection cannot block
                // the next request.
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let index = counter.fetch_add(1, Ordering::SeqCst);
                    let response = &script[index.min(script.len() - 1)];
                    let _ = socket.write_all(response).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (address, served)
    }

    fn client_for(address: SocketAddr) -> AwsS3Client {
        let sdk = aws_config::SdkConfig::builder()
            .behavior_version(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-east-1"))
            .credentials_provider(
                aws_credential_types::provider::SharedCredentialsProvider::new(
                    aws_credential_types::Credentials::new(
                        "AKIDTEST",
                        "SECRETTEST",
                        None,
                        None,
                        "get-retry-test",
                    ),
                ),
            )
            .build();
        AwsS3Client::from_sdk_config_with_options(
            &sdk,
            AwsS3ClientOptions {
                endpoint_url: Some(format!("http://{address}")),
                force_path_style: true,
            },
        )
    }

    #[tokio::test]
    async fn aws_client_reissues_a_get_whose_body_was_cut_off() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let (address, served) = spawn_stub(vec![
                raw_response("etag-1", LOOPBACK_BODY, Some(20)),
                raw_response("etag-2", LOOPBACK_BODY, None),
            ])
            .await;
            let client = client_for(address);

            let data = client
                .get_object("bucket", LOOPBACK_KEY)
                .await
                .expect("the re-issued GET succeeds")
                .expect("the object exists");

            assert_eq!(data.bytes, LOOPBACK_BODY);
            assert_eq!(data.metadata.etag.as_deref(), Some("\"etag-2\""));
            assert_eq!(data.metadata.size, LOOPBACK_BODY.len() as i64);
            assert_eq!(served.load(Ordering::SeqCst), 2);
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn aws_client_reports_unavailable_when_every_body_read_is_cut_off() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let (address, served) =
                spawn_stub(vec![raw_response("etag-1", LOOPBACK_BODY, Some(20))]).await;
            let client = client_for(address);

            let error = client
                .get_object("bucket", LOOPBACK_KEY)
                .await
                .expect_err("every attempt was cut off");

            match error {
                S3ClientError::Unavailable(message) => assert!(
                    message.starts_with("failed to read object body after 3 attempts"),
                    "unexpected message: {message}"
                ),
                other => panic!("expected Unavailable, got {other:?}"),
            }
            // The SDK did not retry the body phase, and HFS made 3 attempts.
            assert_eq!(served.load(Ordering::SeqCst), 3);
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn aws_client_does_not_retry_a_missing_key() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let (address, served) = spawn_stub(vec![raw_not_found()]).await;
            let client = client_for(address);

            let found = client
                .get_object("bucket", LOOPBACK_KEY)
                .await
                .expect("an absent object is not an error");

            assert!(found.is_none());
            assert_eq!(served.load(Ordering::SeqCst), 1);
        })
        .await
        .expect("timed out");
    }
}
