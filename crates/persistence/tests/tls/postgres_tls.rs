//! PostgreSQL connections over TLS against a real server (#2746).
//!
//! Before #2746 every connection was cleartext whatever `sslmode` said. This
//! starts a server with TLS on (the Debian image's snakeoil certificate, which
//! no CA signs) and checks, from the server's own `pg_stat_ssl`, that each mode
//! encrypts or not as libpq would, and that the verify modes refuse a
//! certificate they cannot trust.

use std::path::PathBuf;

use helios_persistence::backends::postgres::{
    PostgresBackend, PostgresConfig, PostgresSslMode, PostgresTlsSettings,
};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;

struct TlsPg {
    host: String,
    port: u16,
    _container: testcontainers::ContainerAsync<Postgres>,
}

static TLS_PG: OnceCell<TlsPg> = OnceCell::const_new();

async fn tls_pg() -> &'static TlsPg {
    TLS_PG
        .get_or_init(|| async {
            let run_id = std::env::var("GITHUB_RUN_ID").unwrap_or_default();
            // The Debian image (not alpine) ships the snakeoil pair, readable by
            // the postgres user through the ssl-cert group.
            let container = super::container_cleanup::with_cleanup_label(
                Postgres::default()
                    .with_tag("16")
                    .with_label("github.run_id", &run_id)
                    .with_cmd([
                        "postgres",
                        "-c",
                        "ssl=on",
                        "-c",
                        "ssl_cert_file=/etc/ssl/certs/ssl-cert-snakeoil.pem",
                        "-c",
                        "ssl_key_file=/etc/ssl/private/ssl-cert-snakeoil.key",
                    ]),
            )
            .start()
            .await
            .expect("Failed to start PostgreSQL container with TLS");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("Failed to get host port");
            let host = container
                .get_host()
                .await
                .expect("Failed to get host")
                .to_string();
            TlsPg {
                host,
                port,
                _container: container,
            }
        })
        .await
}

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pg_tls")
        .join(name)
        .display()
        .to_string()
}

fn config(pg: &TlsPg, settings: &PostgresTlsSettings) -> PostgresConfig {
    PostgresConfig {
        host: pg.host.clone(),
        port: pg.port,
        dbname: "postgres".to_string(),
        user: "postgres".to_string(),
        password: Some("postgres".to_string()),
        max_connections: 2,
        ssl_mode: settings.mode,
        ssl_root_cert: settings.root_cert.clone(),
        ssl_cert: settings.cert.clone(),
        ssl_key: settings.key.clone(),
        ..Default::default()
    }
}

/// Whether a connection made with `settings`' connector is encrypted, as the
/// server reports it.
async fn encrypted(pg: &TlsPg, settings: &PostgresTlsSettings) -> Result<bool, String> {
    let mut pg_config = tokio_postgres::Config::new();
    pg_config
        .host(&pg.host)
        .port(pg.port)
        .dbname("postgres")
        .user("postgres")
        .password("postgres")
        .ssl_mode(match settings.mode {
            PostgresSslMode::Disable => tokio_postgres::config::SslMode::Disable,
            PostgresSslMode::Allow | PostgresSslMode::Prefer => {
                tokio_postgres::config::SslMode::Prefer
            }
            _ => tokio_postgres::config::SslMode::Require,
        });
    let (client, connection) = pg_config
        .connect(settings.connector()?)
        .await
        .map_err(|e| format!("{e:?}"))?;
    tokio::spawn(connection);
    let row = client
        .query_one(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.get(0))
}

#[tokio::test]
async fn postgres_integration_tls_modes_encrypt_as_libpq_would() {
    let pg = tls_pg().await;
    for (mode, expect_tls) in [
        (PostgresSslMode::Disable, false),
        (PostgresSslMode::Allow, true),
        (PostgresSslMode::Prefer, true),
        (PostgresSslMode::Require, true),
    ] {
        let settings = PostgresTlsSettings {
            mode,
            ..Default::default()
        };
        assert_eq!(
            encrypted(pg, &settings).await.unwrap(),
            expect_tls,
            "sslmode={}",
            mode.as_str()
        );
        // The backend's own pool connects with the same settings.
        PostgresBackend::new(config(pg, &settings))
            .await
            .unwrap_or_else(|e| panic!("sslmode={}: {e}", mode.as_str()));
    }
}

#[tokio::test]
async fn postgres_integration_verify_modes_refuse_an_untrusted_certificate() {
    let pg = tls_pg().await;
    // The snakeoil certificate is self-signed; the fixture CA did not sign it.
    for mode in [
        PostgresSslMode::Require,
        PostgresSslMode::VerifyCa,
        PostgresSslMode::VerifyFull,
    ] {
        let settings = PostgresTlsSettings {
            mode,
            root_cert: Some(fixture("ca.pem")),
            ..Default::default()
        };
        let err = encrypted(pg, &settings).await.unwrap_err();
        assert!(
            err.contains("UnknownIssuer"),
            "sslmode={}: {err}",
            mode.as_str()
        );
        let Err(err) = PostgresBackend::new(config(pg, &settings)).await else {
            panic!(
                "sslmode={} accepted an untrusted certificate",
                mode.as_str()
            );
        };
        let text = err.to_string();
        assert!(
            text.contains("connection failed to postgres") && text.contains("UnknownIssuer"),
            "sslmode={}: {text}",
            mode.as_str()
        );
    }
}
