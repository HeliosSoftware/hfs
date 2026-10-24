//! Database locations safe to include in logs.
//!
//! Parse with the database driver and reconstruct only the scheme, hosts, ports,
//! and explicit database name. Never format a configuration or parser error:
//! either can contain credentials. Parsing here does not connect or resolve DNS.

#[cfg(any(feature = "postgres", feature = "mongodb"))]
use std::fmt::Write;
#[cfg(any(feature = "postgres", feature = "mongodb"))]
use std::net::Ipv6Addr;

#[cfg(feature = "mongodb")]
use mongodb::options::{ConnectionString, HostInfo, ServerAddress};
#[cfg(feature = "postgres")]
use tokio_postgres::{Config, config::Host};

const REDACTED: &str = "[redacted]";
#[cfg(feature = "postgres")]
const DEFAULT_POSTGRES_PORT: u16 = 5432;

/// Return a database location without credentials or connection options.
///
/// PostgreSQL accepts both URI and keyword/value syntax. MongoDB parsing is
/// synchronous, including SRV URIs: no DNS lookup is performed. Unknown backends,
/// disabled drivers, and invalid inputs are replaced wholesale. SQLite paths
/// keep the existing URL helper behavior, including masking URI credentials and
/// query/fragment tails when present.
///
/// Raw URI fragments and multiple raw PostgreSQL `@` separators are ambiguous
/// to the drivers (which may treat secrets as host/database content), so those
/// inputs are replaced wholesale too.
pub fn redact_database_url(input: &str, backend: &str) -> String {
    match backend {
        "sqlite" => super::bulk_submit_input::redact_url(input),
        #[cfg(feature = "postgres")]
        "postgres" => postgres_location(input).unwrap_or_else(|| REDACTED.to_owned()),
        #[cfg(feature = "mongodb")]
        "mongodb" => mongodb_location(input).unwrap_or_else(|| REDACTED.to_owned()),
        _ => REDACTED.to_owned(),
    }
}

/// Return the PostgreSQL server a connection string was resolved to, for logs.
///
/// `host`, `port` and `dbname` are what the caller parsed out of `url` and will
/// connect with. Logging them rather than the driver's reading of `url` keeps
/// the line truthful when the caller's parser and the driver's disagree, or
/// when environment overrides apply. If the driver finds `url` invalid or
/// ambiguous, the whole value is redacted: a caller's parser may then have
/// read part of a password as the host or database.
#[cfg(feature = "postgres")]
pub fn redact_postgres_location(url: &str, host: &str, port: u16, dbname: &str) -> String {
    if postgres_location(url).is_none() {
        return REDACTED.to_owned();
    }
    let scheme = if url.starts_with("postgresql://") {
        "postgresql"
    } else {
        "postgres"
    };
    location(
        scheme,
        &[format!("{}:{port}", encode_host(host))],
        Some(dbname),
    )
}

#[cfg(feature = "postgres")]
fn postgres_location(input: &str) -> Option<String> {
    let uri = input.starts_with("postgres://") || input.starts_with("postgresql://");
    if uri && (input.contains('#') || input.matches('@').count() > 1) {
        return None;
    }
    let config: Config = input.parse().ok()?;
    let scheme = if input.starts_with("postgresql://") {
        "postgresql"
    } else {
        "postgres"
    };
    // `(location, is_tcp)`: a port only means something for a TCP host here.
    let mut hosts = config
        .get_hosts()
        .iter()
        .map(|host| match host {
            Host::Tcp(host) => Some((encode_host(host), true)),
            #[cfg(unix)]
            Host::Unix(path) => Some((encode_component(path.to_str()?), false)),
        })
        .collect::<Option<Vec<_>>>()?;
    if hosts.is_empty() {
        hosts = config
            .get_hostaddrs()
            .iter()
            .map(|address| (encode_host(&address.to_string()), true))
            .collect();
    }
    let ports = config.get_ports();
    for (index, (host, tcp)) in hosts.iter_mut().enumerate() {
        // A single configured port applies to every host. With none, show the
        // driver default so URI and keyword/value strings log alike.
        let port = match ports.len() {
            0 if *tcp => Some(&DEFAULT_POSTGRES_PORT),
            1 => ports.first(),
            _ => ports.get(index),
        };
        if let Some(port) = port {
            write!(host, ":{port}").ok()?;
        }
    }
    let hosts = hosts.into_iter().map(|(host, _)| host).collect::<Vec<_>>();
    Some(location(scheme, &hosts, config.get_dbname()))
}

#[cfg(feature = "mongodb")]
fn mongodb_location(input: &str) -> Option<String> {
    if input.contains('#') {
        return None;
    }
    let config = ConnectionString::parse(input).ok()?;
    let (scheme, hosts) = match &config.host_info {
        HostInfo::HostIdentifiers(addresses) => {
            let hosts = addresses
                .iter()
                .map(|address| match address {
                    ServerAddress::Tcp { host, port } => {
                        let mut host = encode_host(host);
                        if let Some(port) = port {
                            write!(host, ":{port}").ok()?;
                        }
                        Some(host)
                    }
                    #[cfg(unix)]
                    ServerAddress::Unix { path } => Some(encode_component(path.to_str()?)),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            ("mongodb", hosts)
        }
        HostInfo::DnsRecord(host) => ("mongodb+srv", vec![encode_host(host)]),
        _ => return None,
    };
    Some(location(scheme, &hosts, config.default_database.as_deref()))
}

#[cfg(any(feature = "postgres", feature = "mongodb"))]
fn location(scheme: &str, hosts: &[String], database: Option<&str>) -> String {
    let mut output = format!("{scheme}://{}", hosts.join(","));
    if let Some(database) = database {
        output.push('/');
        output.push_str(&encode_component(database));
    }
    output
}

#[cfg(any(feature = "postgres", feature = "mongodb"))]
fn encode_host(host: &str) -> String {
    match host.parse::<Ipv6Addr>() {
        Ok(address) => format!("[{address}]"),
        Err(_) => encode_component(host),
    }
}

/// Keep only RFC 3986 unreserved bytes literal. This prevents decoded user data
/// from introducing userinfo, options, fragments, extra hosts, or log newlines.
#[cfg(any(feature = "postgres", feature = "mongodb"))]
fn encode_component(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            output.push('%');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 0xf) as usize]));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::redact_database_url;

    #[test]
    fn sqlite_paths_are_preserved_and_unknown_backends_are_redacted() {
        assert_eq!(
            redact_database_url("./data/hts.db", "sqlite"),
            "./data/hts.db"
        );
        assert_eq!(redact_database_url(":memory:", "sqlite"), ":memory:");
        assert_eq!(
            redact_database_url("file.db?key=secret", "sqlite"),
            "file.db?[redacted]"
        );
        assert_eq!(
            redact_database_url("user:secret@host", "unknown"),
            "[redacted]"
        );
    }

    #[cfg(not(feature = "postgres"))]
    #[test]
    fn disabled_postgres_driver_is_redacted() {
        assert_eq!(
            redact_database_url("password=secret", "postgres"),
            "[redacted]"
        );
    }

    #[cfg(not(feature = "mongodb"))]
    #[test]
    fn disabled_mongodb_driver_is_redacted() {
        assert_eq!(
            redact_database_url("mongodb://user:secret@host/db", "mongodb"),
            "[redacted]"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_credentials_and_options_are_omitted() {
        for (input, expected) in [
            (
                "postgresql://user:secret@db:5432/hts?sslmode=require",
                "postgresql://db:5432/hts",
            ),
            ("postgres://user:p%40ss@db/hts", "postgres://db:5432/hts"),
            (
                "host=db port=5433 user=user password='secret with spaces' dbname=hts application_name=private",
                "postgres://db:5433/hts",
            ),
            (
                "host=db user=user password='escaped\\'secret' dbname=hts",
                "postgres://db:5432/hts",
            ),
            (
                "postgres://user:secret/with/slashes@db:5432/hts",
                "postgres://db:5432/hts",
            ),
            (
                "postgres://user:secret?with?marks@db:5432/hts",
                "postgres://db:5432/hts",
            ),
            (
                "postgres://db:5432/hts?user=user&password=secret&options=private",
                "postgres://db:5432/hts",
            ),
            (
                "host=db-0,db-1 port=5433 user=user password=secret dbname=hts",
                "postgres://db-0:5433,db-1:5433/hts",
            ),
            (
                "hostaddr=127.0.0.1 port=5432 password=secret dbname=hts",
                "postgres://127.0.0.1:5432/hts",
            ),
            (
                "postgres://user:secret@[::1]:5432/hts",
                "postgres://[::1]:5432/hts",
            ),
        ] {
            assert_eq!(redact_database_url(input, "postgres"), expected);
        }
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_retained_components_cannot_introduce_credentials_or_log_lines() {
        assert_eq!(
            redact_database_url(
                "host='db@other,host' password=secret dbname='db?query#fragment\nline'",
                "postgres"
            ),
            "postgres://db%40other:5432,host:5432/db%3Fquery%23fragment%0Aline"
        );
        #[cfg(unix)]
        assert_eq!(
            redact_database_url(
                "host=/var/run/postgresql password=secret dbname=hts",
                "postgres"
            ),
            "postgres://%2Fvar%2Frun%2Fpostgresql/hts"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn invalid_or_fragment_bearing_postgres_inputs_are_redacted_wholesale() {
        for input in [
            "password='unterminated secret",
            "postgres://user:secret@db:invalid/hts",
            "postgres://user:secret@db/hts#private",
            "postgres://user:secret#suffix@db/hts",
            "postgres://user:prefix@secret@db/hts",
        ] {
            assert_eq!(redact_database_url(input, "postgres"), "[redacted]");
        }
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_location_names_the_resolved_server() {
        use super::redact_postgres_location;

        // The caller's parser stopped at `?` and fell back to its defaults:
        // the line names that server, not the one the driver would read.
        assert_eq!(
            redact_postgres_location(
                "postgresql://user:secret?suffix@db:5433/hts",
                "localhost",
                5432,
                "helios"
            ),
            "postgresql://localhost:5432/helios"
        );
        assert_eq!(
            redact_postgres_location("postgres://user:secret@db/hts", "db@x", 5432, "a/b"),
            "postgres://db%40x:5432/a%2Fb"
        );
        // Ambiguous to the driver: the caller may have read the password's
        // tail as the host.
        assert_eq!(
            redact_postgres_location(
                "postgres://user:prefix@secret@db/hts",
                "secret@db",
                5432,
                "hts"
            ),
            "[redacted]"
        );
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn mongodb_credentials_and_options_are_omitted_without_resolving_hosts() {
        for (input, expected) in [
            (
                "mongodb://user:p%40ss@db-0:27017,db-1:27017/helios?authSource=admin&replicaSet=rs0",
                "mongodb://db-0:27017,db-1:27017/helios",
            ),
            (
                "mongodb://user:secret@db-0:27017,db-1:27017?replicaSet=rs0",
                "mongodb://db-0:27017,db-1:27017",
            ),
            (
                "mongodb+srv://user:secret@cluster.example.invalid/helios?authSource=admin",
                "mongodb+srv://cluster.example.invalid/helios",
            ),
            (
                "mongodb://user:secret@[::1]:27017/helios",
                "mongodb://[::1]:27017/helios",
            ),
            ("mongodb://db/helios", "mongodb://db/helios"),
        ] {
            assert_eq!(redact_database_url(input, "mongodb"), expected);
        }
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn invalid_mongodb_inputs_are_redacted_wholesale() {
        for input in [
            "user:secret@db/helios",
            "mongodb:/user:secret@db/helios",
            "mongodb://user:secret/slash@db/helios",
            "mongodb://user:secret@db:invalid/helios",
            "mongodb://user:secret@db/helios#private",
        ] {
            assert_eq!(redact_database_url(input, "mongodb"), "[redacted]");
        }
    }
}
