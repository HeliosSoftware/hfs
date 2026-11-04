//! Test-only wiremock wrapper whose requests are told apart from stray traffic (#2481).
//!
//! The terminology tests assert on how many requests a stub received. A bare
//! `MockServer` counts every request that reaches its port, including ones that
//! are not the test's own: wiremock hands pooled servers from test to test, and
//! another test process (a parallel worktree, say) may still be sending to a port
//! the OS has since given to this stub. Those strays made the retry tests see,
//! say, 5 requests where the client sent 4.
//!
//! A [`StubServer`] puts a unique path prefix in front of every route. The client
//! is given [`StubServer::uri`], which carries the prefix; mocks match only
//! prefixed paths, so a stray request gets wiremock's 404 and not a scripted
//! answer; and [`StubServer::requests`] counts only prefixed requests, printing
//! any other one so a future stray can be traced to its source.

use std::ops::Deref;
use std::sync::atomic::{AtomicUsize, Ordering};

use wiremock::matchers::{PathExactMatcher, PathRegexMatcher, path, path_regex};
use wiremock::{MockServer, Request};

/// A wiremock server that serves, and counts, only paths under its own prefix.
///
/// Dereferences to the inner [`MockServer`] so `Mock::mount(&stub)` works, but
/// use [`StubServer::uri`] and [`StubServer::requests`] rather than the inner
/// server's: those see stray traffic too.
pub(crate) struct StubServer {
    server: MockServer,
    prefix: String,
}

impl StubServer {
    /// Starts a stub with a prefix unique to this process and this stub.
    pub(crate) async fn start() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let prefix = format!(
            "/stub-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        Self {
            server: MockServer::start().await,
            prefix,
        }
    }

    /// Base URL to hand to the client under test: the server's address plus the prefix.
    pub(crate) fn uri(&self) -> String {
        format!("{}{}", self.server.uri(), self.prefix)
    }

    /// Matches `route` (e.g. `/ValueSet/$expand`) under this stub's prefix.
    pub(crate) fn path(&self, route: &str) -> PathExactMatcher {
        path(format!("{}{route}", self.prefix))
    }

    /// Matches any path under this stub's prefix.
    pub(crate) fn any_path(&self) -> PathRegexMatcher {
        path_regex(format!("^{}/", regex::escape(&self.prefix)))
    }

    fn is_own(&self, request: &Request) -> bool {
        request
            .url
            .path()
            .strip_prefix(&self.prefix)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// The requests the client under test sent, in arrival order.
    ///
    /// Requests without this stub's prefix are left out and printed to stderr,
    /// with their method, URL, headers and body.
    pub(crate) async fn requests(&self) -> Vec<Request> {
        let (own, stray): (Vec<_>, Vec<_>) = self
            .server
            .received_requests()
            .await
            .expect("request recording is on by default")
            .into_iter()
            .partition(|request| self.is_own(request));
        for request in &stray {
            eprintln!(
                "stub {} ignored a stray request (#2481): {} {}\nheaders: {:?}\nbody: {}",
                self.prefix,
                request.method,
                request.url,
                request.headers,
                String::from_utf8_lossy(&request.body)
            );
        }
        own
    }

    /// How many requests the client under test sent; see [`StubServer::requests`].
    pub(crate) async fn request_count(&self) -> usize {
        self.requests().await.len()
    }
}

impl Deref for StubServer {
    type Target = MockServer;

    fn deref(&self) -> &MockServer {
        &self.server
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{Mock, ResponseTemplate};

    #[tokio::test]
    async fn requests_outside_the_prefix_are_neither_answered_nor_counted() {
        let stub = StubServer::start().await;
        Mock::given(stub.any_path())
            .respond_with(ResponseTemplate::new(200))
            .mount(&stub)
            .await;

        let client = reqwest::Client::new();
        let own = client
            .get(format!("{}/ValueSet/$expand", stub.uri()))
            .send()
            .await
            .unwrap();
        assert_eq!(own.status(), 200);

        // a stray: same port, no prefix
        let stray = client
            .get(format!("{}/ValueSet/$expand", stub.server.uri()))
            .send()
            .await
            .unwrap();
        assert_eq!(stray.status(), 404);

        // another stub's prefix that merely starts with this one's
        let longer = client
            .get(format!("{}0/ValueSet/$expand", stub.uri()))
            .send()
            .await
            .unwrap();
        assert_eq!(longer.status(), 404);

        let requests = stub.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url.path(),
            format!("{}/ValueSet/$expand", stub.prefix)
        );
    }
}
