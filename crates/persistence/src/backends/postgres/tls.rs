//! TLS for PostgreSQL connections (#2746).
//!
//! Shared by the HFS PostgreSQL backend and the HTS terminology server so both
//! read the libpq connection parameters the same way and build the same
//! connector:
//!
//! | `sslmode`     | Encryption                         | Server certificate                    |
//! |---------------|------------------------------------|---------------------------------------|
//! | `disable`     | never                              | —                                     |
//! | `allow`       | when the server offers it          | not verified                          |
//! | `prefer`      | when the server offers it          | not verified                          |
//! | `require`     | always                             | not verified, unless `sslrootcert` is set (then as `verify-ca`) |
//! | `verify-ca`   | always                             | chain to a trusted root               |
//! | `verify-full` | always                             | chain to a trusted root and host name |
//!
//! These are libpq's meanings, with one difference: `allow` tries TLS first
//! (libpq tries cleartext first), because the driver cannot start in cleartext
//! and upgrade. The result is never weaker than libpq's.
//!
//! The trusted roots are `sslrootcert`: a PEM file, or `system` for the
//! platform's store. Unset, the verify modes use the platform's store (libpq
//! reads `~/.postgresql/root.crt` instead). `sslcert` and `sslkey` give a client
//! certificate for `cert` authentication.
//!
//! When the server presents a certificate, tokio-postgres also uses SCRAM
//! channel binding where the server supports it, so even `require` without
//! verification ties the password exchange to the TLS session.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use serde::{Deserialize, Serialize};
use tokio_postgres_rustls::MakeRustlsConnect;

/// The TLS connector type the PostgreSQL pools are built with.
pub type PostgresTlsConnector = MakeRustlsConnect;

/// SSL mode for PostgreSQL connections, named as libpq names them.
///
/// Declared from weakest to strongest, so `Ord` ranks them: a configured
/// minimum is applied with `max` (see `HFS_PG_SSL_MODE`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum PostgresSslMode {
    /// Never encrypt.
    Disable,
    /// Encrypt when the server offers TLS; certificate not verified. Tried
    /// TLS-first, unlike libpq, which tries cleartext first.
    Allow,
    /// Encrypt when the server offers TLS; certificate not verified.
    #[default]
    Prefer,
    /// Always encrypt; certificate not verified unless `sslrootcert` is set.
    Require,
    /// Always encrypt; certificate must chain to a trusted root.
    VerifyCa,
    /// Always encrypt; certificate must chain to a trusted root and name the host.
    VerifyFull,
}

impl PostgresSslMode {
    /// Every mode, weakest first; the spelling list in error messages.
    pub const ALL: [Self; 6] = [
        Self::Disable,
        Self::Allow,
        Self::Prefer,
        Self::Require,
        Self::VerifyCa,
        Self::VerifyFull,
    ];

    /// Parses a libpq `sslmode` value (case-insensitive); `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str().eq_ignore_ascii_case(value))
    }

    /// The libpq spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Allow => "allow",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }

    /// Whether the mode refuses a cleartext connection.
    pub fn wants_tls(self) -> bool {
        self >= Self::Require
    }

    /// The driver's mode: it knows only disable / prefer / require; the
    /// certificate checks live in the connector.
    pub fn driver_mode(self) -> deadpool_postgres::SslMode {
        match self {
            Self::Disable => deadpool_postgres::SslMode::Disable,
            Self::Allow | Self::Prefer => deadpool_postgres::SslMode::Prefer,
            Self::Require | Self::VerifyCa | Self::VerifyFull => {
                deadpool_postgres::SslMode::Require
            }
        }
    }

    /// `"disable, allow, prefer, require, verify-ca, verify-full"`.
    pub fn spellings() -> String {
        Self::ALL.map(Self::as_str).join(", ")
    }
}

/// How much of the server's certificate is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertVerification {
    /// Not checked (or no TLS at all).
    None,
    /// Must chain to a trusted root; the host name is not checked.
    Ca,
    /// Must chain to a trusted root and name the host.
    Full,
}

impl CertVerification {
    /// Log spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ca => "ca",
            Self::Full => "full",
        }
    }
}

/// The `sslrootcert` value naming the platform's trust store (libpq 16).
pub const SYSTEM_ROOT_CERT: &str = "system";

/// The libpq TLS parameters of one connection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresTlsSettings {
    /// `sslmode`.
    #[serde(default)]
    pub mode: PostgresSslMode,
    /// `sslrootcert`: a PEM file of trusted roots, or `system`.
    #[serde(default)]
    pub root_cert: Option<String>,
    /// `sslcert`: the client certificate (PEM), with `key`.
    #[serde(default)]
    pub cert: Option<PathBuf>,
    /// `sslkey`: the client certificate's private key (PEM), with `cert`.
    #[serde(default)]
    pub key: Option<PathBuf>,
}

impl PostgresTlsSettings {
    /// How much of the server certificate the connector checks.
    pub fn verification(&self) -> CertVerification {
        match self.mode {
            PostgresSslMode::Disable | PostgresSslMode::Allow | PostgresSslMode::Prefer => {
                CertVerification::None
            }
            // libpq: "if a root CA file exists, the behavior of
            // sslmode=require will be the same as that of verify-ca".
            PostgresSslMode::Require if self.root_cert.is_some() => CertVerification::Ca,
            PostgresSslMode::Require => CertVerification::None,
            PostgresSslMode::VerifyCa => CertVerification::Ca,
            PostgresSslMode::VerifyFull => CertVerification::Full,
        }
    }

    /// Builds the connector, reading the certificate files. The error names
    /// what is wrong in words fit for a startup failure.
    pub fn connector(&self) -> Result<PostgresTlsConnector, String> {
        if self.root_cert.as_deref() == Some(SYSTEM_ROOT_CERT)
            && self.mode != PostgresSslMode::VerifyFull
        {
            // libpq refuses this too: trusting every public CA is only
            // meaningful when the host name is checked as well.
            return Err(format!(
                "sslrootcert=system needs sslmode=verify-full, not sslmode={}",
                self.mode.as_str()
            ));
        }
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verification = self.verification();
        let roots = match verification {
            CertVerification::None => None,
            CertVerification::Ca | CertVerification::Full => Some(Arc::new(self.load_roots()?)),
        };
        let verifier = Arc::new(PostgresServerVerifier {
            roots,
            check_name: verification == CertVerification::Full,
            provider: Arc::clone(&provider),
        });
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS configuration: {e}"))?
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        let config = match (&self.cert, &self.key) {
            (Some(cert), Some(key)) => {
                let chain = CertificateDer::pem_file_iter(cert)
                    .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
                    .map_err(|e| format!("sslcert {}: {e}", cert.display()))?;
                if chain.is_empty() {
                    return Err(format!(
                        "sslcert {}: no PEM certificate in the file",
                        cert.display()
                    ));
                }
                let key_der = PrivateKeyDer::from_pem_file(key)
                    .map_err(|e| format!("sslkey {}: {e}", key.display()))?;
                builder
                    .with_client_auth_cert(chain, key_der)
                    .map_err(|e| format!("sslcert/sslkey: {e}"))?
            }
            (None, None) => builder.with_no_client_auth(),
            (Some(_), None) => return Err("sslcert is set without sslkey".to_string()),
            (None, Some(_)) => return Err("sslkey is set without sslcert".to_string()),
        };
        Ok(MakeRustlsConnect::new(config))
    }

    fn load_roots(&self) -> Result<RootCertStore, String> {
        let mut roots = RootCertStore::empty();
        match self.root_cert.as_deref() {
            Some(path) if path != SYSTEM_ROOT_CERT => {
                let certs = CertificateDer::pem_file_iter(Path::new(path))
                    .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
                    .map_err(|e| format!("sslrootcert {path}: {e}"))?;
                let (added, _ignored) = roots.add_parsable_certificates(certs);
                if added == 0 {
                    return Err(format!(
                        "sslrootcert {path}: no usable PEM certificate in the file"
                    ));
                }
            }
            _ => {
                let native = rustls_native_certs::load_native_certs();
                roots.add_parsable_certificates(native.certs);
                if roots.is_empty() {
                    let errors: Vec<String> = native.errors.iter().map(|e| e.to_string()).collect();
                    return Err(format!(
                        "sslmode={} verifies the server certificate, but the platform trust store \
                         has no usable certificate{}; set sslrootcert to a PEM file of the \
                         database's CA",
                        self.mode.as_str(),
                        if errors.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", errors.join("; "))
                        }
                    ));
                }
            }
        }
        Ok(roots)
    }
}

/// Checks the server certificate as far as the `sslmode` asks, and always
/// checks the handshake signatures, so the session (and the channel binding
/// SCRAM derives from it) belongs to the key in the presented certificate.
#[derive(Debug)]
struct PostgresServerVerifier {
    /// `None`: the chain is not checked (`prefer`, `require`).
    roots: Option<Arc<RootCertStore>>,
    check_name: bool,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PostgresServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(roots) = &self.roots {
            let cert = ParsedCertificate::try_from(end_entity)?;
            rustls::client::verify_server_cert_signed_by_trust_anchor(
                &cert,
                roots,
                intermediates,
                now,
                self.provider.signature_verification_algorithms.all,
            )?;
            if self.check_name {
                rustls::client::verify_server_name(&cert, server_name)?;
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The TLS parameters found in a connection URL's query string. `None` where
/// the URL does not set one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UrlTlsParams {
    /// `sslmode`.
    pub mode: Option<PostgresSslMode>,
    /// `sslrootcert`.
    pub root_cert: Option<String>,
    /// `sslcert`.
    pub cert: Option<PathBuf>,
    /// `sslkey`.
    pub key: Option<PathBuf>,
}

/// The query parameters [`take_tls_params`] reads.
const TLS_PARAMS: [&str; 4] = ["sslmode", "sslrootcert", "sslcert", "sslkey"];

/// Reads `sslmode`, `sslrootcert`, `sslcert` and `sslkey` from a URL query
/// string (without the `?`), percent-decoded. When a parameter repeats, the
/// last one wins, as in libpq. Returns the other `key=value` pairs verbatim.
///
/// `source` names the setting the URL came from (`HFS_DATABASE_URL`, ...) in
/// the error for an unknown `sslmode`.
pub fn take_tls_params<'q>(
    query: &'q str,
    source: &str,
) -> Result<(UrlTlsParams, Vec<&'q str>), String> {
    let mut params = UrlTlsParams::default();
    let mut rest = Vec::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if !TLS_PARAMS.contains(&key) {
            rest.push(pair);
            continue;
        }
        let value = percent_decode(value);
        match key {
            "sslmode" => {
                params.mode = Some(
                    PostgresSslMode::parse(&value)
                        .ok_or_else(|| unknown_ssl_mode_message(source, &value))?,
                );
            }
            "sslrootcert" => params.root_cert = Some(value),
            "sslcert" => params.cert = Some(PathBuf::from(value)),
            _ => params.key = Some(PathBuf::from(value)),
        }
    }
    Ok((params, rest))
}

/// `<source>: sslmode "<value>" is not one of ...`.
pub fn unknown_ssl_mode_message(source: &str, value: &str) -> String {
    format!(
        "{source}: sslmode {value:?} is not one of {}",
        PostgresSslMode::spellings()
    )
}

/// Splits the TLS parameters out of a `postgres://` / `postgresql://` URL and
/// returns the URL without them, for a driver that rejects the parameters it
/// does not know (`sslrootcert`, ...) or the modes it does not have
/// (`verify-ca`, ...). Any other string (a `key=value` connection string) is
/// returned unchanged with no parameters read.
pub fn split_tls_params(url: &str, source: &str) -> Result<(String, UrlTlsParams), String> {
    let is_url = url.starts_with("postgres://") || url.starts_with("postgresql://");
    let Some((base, query)) = url.split_once('?').filter(|_| is_url) else {
        return Ok((url.to_string(), UrlTlsParams::default()));
    };
    let (params, rest) = take_tls_params(query, source)?;
    let stripped = if rest.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", rest.join("&"))
    };
    Ok((stripped, params))
}

/// Decodes `%XX` escapes (and nothing else: a `+` stays a `+`, as libpq reads
/// a URI). An invalid escape is kept as written.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Logs, once per process for each `target` and mode, whether the connection
/// `client` holds is encrypted, read from `pg_stat_ssl` on the server.
///
/// `disable` is logged at INFO: cleartext was asked for. `allow` / `prefer`
/// falling back to cleartext is a WARN, since the operator may not know the
/// server refused TLS. A failed probe is logged at DEBUG only.
pub async fn log_connection_security(
    client: &tokio_postgres::Client,
    service: &str,
    target: &str,
    settings: &PostgresTlsSettings,
) {
    static LOGGED: OnceLock<Mutex<HashSet<(String, String, PostgresSslMode)>>> = OnceLock::new();
    let key = (service.to_string(), target.to_string(), settings.mode);
    {
        let mut logged = LOGGED
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !logged.insert(key) {
            return;
        }
    }
    let sslmode = settings.mode.as_str();
    let probe = client
        .query_opt(
            "SELECT ssl, version FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await;
    let row = match probe {
        Ok(Some(row)) => row,
        Ok(None) => {
            tracing::debug!(
                service,
                target,
                sslmode,
                "pg_stat_ssl has no row for this session"
            );
            return;
        }
        Err(e) => {
            tracing::debug!(service, target, sslmode, error = %e, "could not read pg_stat_ssl");
            return;
        }
    };
    let encrypted: bool = row.try_get(0).unwrap_or(false);
    if encrypted {
        let tls_version: Option<String> = row.try_get(1).ok().flatten();
        tracing::info!(
            service,
            target,
            sslmode,
            tls_version = tls_version.as_deref().unwrap_or("unknown"),
            certificate_verification = settings.verification().as_str(),
            "PostgreSQL connection is encrypted"
        );
    } else if settings.mode == PostgresSslMode::Disable {
        tracing::info!(
            service,
            target,
            sslmode,
            "PostgreSQL connection is not encrypted (sslmode=disable)"
        );
    } else {
        tracing::warn!(
            service,
            target,
            sslmode,
            "PostgreSQL connection is not encrypted: the server did not accept TLS; set \
             sslmode=require (or verify-full) to refuse a cleartext connection"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pg_tls")
            .join(name)
    }

    fn server_chain() -> Vec<CertificateDer<'static>> {
        CertificateDer::pem_file_iter(fixture("server.pem"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn verifier(root: &str, check_name: bool) -> PostgresServerVerifier {
        let settings = PostgresTlsSettings {
            mode: PostgresSslMode::VerifyCa,
            root_cert: Some(fixture(root).display().to_string()),
            ..Default::default()
        };
        PostgresServerVerifier {
            roots: Some(Arc::new(settings.load_roots().unwrap())),
            check_name,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }

    fn check(verifier: &PostgresServerVerifier, host: &str) -> Result<(), rustls::Error> {
        let chain = server_chain();
        verifier
            .verify_server_cert(
                &chain[0],
                &chain[1..],
                &ServerName::try_from(host.to_string()).unwrap(),
                &[],
                UnixTime::now(),
            )
            .map(|_| ())
    }

    #[test]
    fn modes_parse_in_any_case_and_rank_weakest_first() {
        for mode in PostgresSslMode::ALL {
            assert_eq!(PostgresSslMode::parse(mode.as_str()), Some(mode));
            assert_eq!(
                PostgresSslMode::parse(&mode.as_str().to_ascii_uppercase()),
                Some(mode)
            );
        }
        assert_eq!(PostgresSslMode::parse("requier"), None);
        assert_eq!(PostgresSslMode::parse(""), None);
        assert!(PostgresSslMode::Disable < PostgresSslMode::Allow);
        assert!(PostgresSslMode::Allow < PostgresSslMode::Prefer);
        assert!(PostgresSslMode::Prefer < PostgresSslMode::Require);
        assert!(PostgresSslMode::Require < PostgresSslMode::VerifyCa);
        assert!(PostgresSslMode::VerifyCa < PostgresSslMode::VerifyFull);
        assert!(!PostgresSslMode::Prefer.wants_tls());
        assert!(PostgresSslMode::Require.wants_tls());
    }

    #[test]
    fn verification_follows_libpq() {
        let with = |mode, root: Option<&str>| PostgresTlsSettings {
            mode,
            root_cert: root.map(str::to_string),
            ..Default::default()
        };
        assert_eq!(
            with(PostgresSslMode::Prefer, Some("/ca.pem")).verification(),
            CertVerification::None
        );
        assert_eq!(
            with(PostgresSslMode::Require, None).verification(),
            CertVerification::None
        );
        assert_eq!(
            with(PostgresSslMode::Require, Some("/ca.pem")).verification(),
            CertVerification::Ca
        );
        assert_eq!(
            with(PostgresSslMode::VerifyCa, None).verification(),
            CertVerification::Ca
        );
        assert_eq!(
            with(PostgresSslMode::VerifyFull, None).verification(),
            CertVerification::Full
        );
    }

    #[test]
    fn verify_ca_accepts_the_right_ca_whatever_the_host_name() {
        let ca = verifier("ca.pem", false);
        check(&ca, "localhost").unwrap();
        check(&ca, "db.example.internal").unwrap();
    }

    #[test]
    fn a_certificate_from_another_ca_is_refused() {
        for check_name in [false, true] {
            let err = check(&verifier("other-ca.pem", check_name), "localhost").unwrap_err();
            assert!(
                matches!(err, rustls::Error::InvalidCertificate(_)),
                "{err:?}"
            );
        }
    }

    #[test]
    fn verify_full_checks_the_host_name_and_ip() {
        let full = verifier("ca.pem", true);
        check(&full, "localhost").unwrap();
        check(&full, "127.0.0.1").unwrap();
        let err = check(&full, "db.example.internal").unwrap_err();
        assert!(
            matches!(err, rustls::Error::InvalidCertificate(_)),
            "{err:?}"
        );
        assert!(check(&full, "127.0.0.2").is_err());
    }

    #[test]
    fn no_verification_accepts_any_chain() {
        let none = PostgresServerVerifier {
            roots: None,
            check_name: false,
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        };
        check(&none, "db.example.internal").unwrap();
    }

    #[test]
    fn connector_builds_for_every_mode_and_reads_the_files() {
        for mode in PostgresSslMode::ALL {
            let settings = PostgresTlsSettings {
                mode,
                root_cert: Some(fixture("ca.pem").display().to_string()),
                cert: Some(fixture("client.pem")),
                key: Some(fixture("client.key")),
            };
            settings.connector().unwrap();
        }
    }

    #[test]
    fn connector_refuses_bad_file_settings() {
        let err = |settings: PostgresTlsSettings| settings.connector().err().unwrap();
        let missing = err(PostgresTlsSettings {
            mode: PostgresSslMode::VerifyCa,
            root_cert: Some("/nonexistent/root.crt".to_string()),
            ..Default::default()
        });
        assert!(
            missing.contains("sslrootcert /nonexistent/root.crt"),
            "{missing}"
        );
        let not_pem = err(PostgresTlsSettings {
            mode: PostgresSslMode::VerifyFull,
            root_cert: Some(fixture("client.key").display().to_string()),
            ..Default::default()
        });
        assert!(not_pem.contains("no usable PEM certificate"), "{not_pem}");
        let half = err(PostgresTlsSettings {
            mode: PostgresSslMode::Require,
            cert: Some(fixture("client.pem")),
            ..Default::default()
        });
        assert!(half.contains("sslcert is set without sslkey"), "{half}");
        let system = err(PostgresTlsSettings {
            mode: PostgresSslMode::Require,
            root_cert: Some(SYSTEM_ROOT_CERT.to_string()),
            ..Default::default()
        });
        assert!(system.contains("needs sslmode=verify-full"), "{system}");
    }

    #[test]
    fn tls_params_last_wins_and_are_percent_decoded() {
        let (params, rest) = take_tls_params(
            "sslmode=disable&application_name=x&sslmode=verify%2Dfull&sslrootcert=%2Fetc%2Fca.pem&sslcert=/c.pem&sslkey=/k.pem",
            "HFS_DATABASE_URL",
        )
        .unwrap();
        assert_eq!(params.mode, Some(PostgresSslMode::VerifyFull));
        assert_eq!(params.root_cert.as_deref(), Some("/etc/ca.pem"));
        assert_eq!(params.cert, Some(PathBuf::from("/c.pem")));
        assert_eq!(params.key, Some(PathBuf::from("/k.pem")));
        assert_eq!(rest, vec!["application_name=x"]);

        let err = take_tls_params("sslmode=requier", "HFS_AUDIT_DATABASE_URL").unwrap_err();
        assert!(
            err.starts_with("HFS_AUDIT_DATABASE_URL: sslmode \"requier\""),
            "{err}"
        );
        assert!(err.contains("allow"), "{err}");
    }

    #[test]
    fn split_tls_params_strips_only_the_tls_parameters() {
        let (url, params) = split_tls_params(
            "postgres://u:p@db:5432/hts?application_name=hts&sslmode=verify-ca&sslrootcert=/ca.pem",
            "HTS_DATABASE_URL",
        )
        .unwrap();
        assert_eq!(url, "postgres://u:p@db:5432/hts?application_name=hts");
        assert_eq!(params.mode, Some(PostgresSslMode::VerifyCa));

        let (url, params) =
            split_tls_params("postgresql://u@db/hts?sslmode=require", "HTS_DATABASE_URL").unwrap();
        assert_eq!(url, "postgresql://u@db/hts");
        assert_eq!(params.mode, Some(PostgresSslMode::Require));

        let dsn = "host=db user=u sslmode=require";
        let (url, params) = split_tls_params(dsn, "HTS_DATABASE_URL").unwrap();
        assert_eq!(url, dsn);
        assert_eq!(params, UrlTlsParams::default());
    }
}
