//! The stop signal every Helios server drains on.
//!
//! Ctrl-C (SIGINT) is what a developer sends from a terminal; SIGTERM is what a
//! process supervisor, container runtime, or load-balanced rolling deploy
//! sends. Both must drain in-flight requests the same way. SIGTERM matters more
//! than it looks: the container image `exec`s the server as PID 1, and the
//! kernel discards a signal sent to PID 1 unless the process installed a
//! handler for it — so a server that only listens for Ctrl-C ignores
//! `docker stop` entirely and dies to the SIGKILL at the end of the grace
//! period.
//!
//! ```rust,ignore
//! axum::serve(listener, app)
//!     .with_graceful_shutdown(async {
//!         let signal = helios_observability::shutdown::signal().await;
//!         tracing::info!(signal, "Shutdown signal received, draining connections");
//!     })
//!     .await?;
//! // Flush here, after the drain -- not inside the future above.
//! helios_observability::telemetry::shutdown();
//! ```
//!
//! Do the flushing *after* `serve` returns. axum awaits the whole shutdown
//! future before it stops accepting connections or tells any of them to wind
//! down, so work placed inside it runs while requests are still in flight and
//! misses everything they record.

/// Resolves when the process is asked to stop, returning the signal's name
/// (`"SIGINT"` or `"SIGTERM"`) for the shutdown log line.
///
/// If the SIGTERM handler cannot be installed this warns and keeps Ctrl-C
/// only. Non-Unix targets listen for Ctrl-C only.
pub async fn signal() -> &'static str {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to install the SIGTERM handler; only Ctrl-C will stop this instance"
                );
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => "SIGINT",
        _ = terminate => "SIGTERM",
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A SIGTERM delivered to this process resolves [`signal`] as `"SIGTERM"`
    /// instead of terminating the test binary — i.e. the handler is really
    /// installed. Raised only after the future has been polled once, because
    /// the handler is registered on first poll.
    #[tokio::test]
    async fn sigterm_resolves_the_signal_future() {
        let mut fut = std::pin::pin!(signal());
        assert!(futures_poll_once(fut.as_mut()).await.is_none());
        // SAFETY: `raise` only sends a signal to this process.
        unsafe {
            libc_raise(15);
        }
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), fut)
            .await
            .expect("signal() did not resolve after SIGTERM");
        assert_eq!(got, "SIGTERM");
    }

    async fn futures_poll_once<F: std::future::Future + Unpin>(fut: F) -> Option<F::Output> {
        tokio::select! {
            biased;
            out = fut => Some(out),
            _ = tokio::task::yield_now() => None,
        }
    }

    unsafe extern "C" {
        #[link_name = "raise"]
        fn libc_raise(sig: i32) -> i32;
    }
}
