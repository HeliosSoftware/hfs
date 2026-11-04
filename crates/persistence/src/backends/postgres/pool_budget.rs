//! One connection budget for all the pools a process opens against a server (#2216).
//!
//! A pool whose size came from the default ([`PostgresConfig::limit_pool_to_server`])
//! is limited to half of the server's `max_connections`, so that a second
//! instance during a rolling restart, a migration tool or an operator's `psql`
//! still find free slots. That half is a budget for the process, not for each
//! pool: `hfs` with the database audit sink opens a second pool to the same
//! server, and two pools each taking half filled a stock server
//! (`max_connections = 100`) to the last slot.
//!
//! Pools sized explicitly are used as given, but their sizes come out of the
//! budget first. Every registration and every release splits what is left over
//! the default-sized pools again and resizes them, so the outcome does not
//! depend on which pool was opened first (`hfs` opens the audit pool before
//! the primary one). Pools count as the same server when they were configured
//! with the same host and port.
//!
//! [`PostgresConfig::limit_pool_to_server`]: super::PostgresConfig::limit_pool_to_server

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use deadpool_postgres::Pool;
use parking_lot::Mutex;

type ServerKey = (String, u16);

static SERVERS: LazyLock<Mutex<HashMap<ServerKey, Server>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Server {
    max_connections: usize,
    pools: Vec<Entry>,
}

struct Entry {
    id: u64,
    /// The size the pool was configured with, the most it is ever given.
    configured: usize,
    /// Whether the pool is resized to fit the budget.
    limited: bool,
    pool: Pool,
}

/// Keeps a pool's share of the budget; dropping it hands the share back to
/// the other pools on the same server.
pub(super) struct PoolRegistration {
    key: ServerKey,
    id: u64,
}

impl Drop for PoolRegistration {
    fn drop(&mut self) {
        let mut servers = SERVERS.lock();
        if let Some(server) = servers.get_mut(&self.key) {
            server.pools.retain(|entry| entry.id != self.id);
            if server.pools.is_empty() {
                servers.remove(&self.key);
            } else {
                rebalance(server);
            }
        }
    }
}

/// Registers `pool`, which is configured with `configured` connections, against
/// a server reporting `server_max_connections`, and resizes the default-sized
/// pools on that server, `pool` included when `limited`.
pub(super) fn register(
    host: &str,
    port: u16,
    server_max_connections: usize,
    configured: usize,
    limited: bool,
    pool: &Pool,
) -> PoolRegistration {
    let key = (host.to_string(), port);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let mut servers = SERVERS.lock();
    let server = servers.entry(key.clone()).or_insert_with(|| Server {
        max_connections: server_max_connections,
        pools: Vec::new(),
    });
    server.max_connections = server_max_connections;
    server.pools.push(Entry {
        id,
        configured,
        limited,
        pool: pool.clone(),
    });
    rebalance(server);

    let total: usize = server
        .pools
        .iter()
        .map(|entry| entry.pool.status().max_size)
        .sum();
    if total > server_max_connections {
        tracing::warn!(
            pools = server.pools.len(),
            pool_connections = total,
            server_max_connections,
            "the PostgreSQL pools of this process can open more connections than the \
             server's max_connections; requests will fail with connection errors once \
             the server is full"
        );
    }
    PoolRegistration { key, id }
}

fn rebalance(server: &Server) {
    let explicit: usize = server
        .pools
        .iter()
        .filter(|entry| !entry.limited)
        .map(|entry| entry.configured)
        .sum();
    let limited: Vec<&Entry> = server.pools.iter().filter(|entry| entry.limited).collect();
    let wanted: Vec<usize> = limited.iter().map(|entry| entry.configured).collect();
    let sizes = split_budget(server.max_connections, explicit, &wanted);
    for (entry, size) in limited.into_iter().zip(sizes) {
        if entry.pool.status().max_size == size {
            continue;
        }
        if size < entry.configured {
            tracing::info!(
                configured = entry.configured,
                pool_size = size,
                server_max_connections = server.max_connections,
                pools = server.pools.len(),
                "postgres pool size limited to its share of half of the server's \
                 max_connections"
            );
        }
        entry.pool.resize(size);
    }
}

/// Splits half of `server_max_connections`, less the `explicit` connections
/// of pools sized by hand, over pools wanting `wanted` connections each.
///
/// A pool that wants less than an even share leaves the rest to the others.
/// Every pool keeps at least one connection, even when nothing is left.
fn split_budget(server_max_connections: usize, explicit: usize, wanted: &[usize]) -> Vec<usize> {
    let mut remaining = (server_max_connections / 2).saturating_sub(explicit);
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    order.sort_by_key(|&i| wanted[i]);
    let mut sizes = vec![0; wanted.len()];
    for (served, &i) in order.iter().enumerate() {
        let share = remaining / (wanted.len() - served);
        let size = wanted[i].min(share).max(1);
        sizes[i] = size;
        remaining = remaining.saturating_sub(size);
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::split_budget;

    #[test]
    fn a_single_pool_gets_at_most_half_of_the_server() {
        assert_eq!(split_budget(1000, 0, &[32]), [32]);
        assert_eq!(split_budget(128, 0, &[64]), [64]);
        assert_eq!(split_budget(100, 0, &[64]), [50]);
        assert_eq!(split_budget(100, 0, &[32]), [32]);
        assert_eq!(split_budget(10, 0, &[64]), [5]);
    }

    #[test]
    fn pools_on_one_server_share_the_half() {
        // `hfs` with the database audit sink on a stock server: 25 + 25, not 50 + 50.
        assert_eq!(split_budget(100, 0, &[64, 64]), [25, 25]);
        assert_eq!(split_budget(100, 0, &[32, 32]), [25, 25]);
        // What one pool does not want goes to the other, in either order.
        assert_eq!(split_budget(100, 0, &[10, 64]), [10, 40]);
        assert_eq!(split_budget(100, 0, &[64, 10]), [40, 10]);
        assert_eq!(split_budget(1000, 0, &[32, 32]), [32, 32]);
    }

    #[test]
    fn explicit_pools_come_out_of_the_budget_first() {
        assert_eq!(split_budget(100, 40, &[32]), [10]);
        assert_eq!(split_budget(100, 40, &[32, 32]), [5, 5]);
    }

    #[test]
    fn every_pool_keeps_one_connection() {
        assert_eq!(split_budget(100, 60, &[32]), [1]);
        assert_eq!(split_budget(1, 0, &[64]), [1]);
        assert_eq!(split_budget(0, 0, &[16, 16]), [1, 1]);
    }
}
