//! The driver's topology (SDAM) events forget a backend's kept transactional
//! `hello` answer (#2411, #2308).
//!
//! A backend keeps the transactional answer of a write's topology check
//! ([`WriteTopologyMemo`]) and stops asking before each write. Without this
//! module it forgot the answer only when a write reported a problem, so the
//! first write after a replica set came back as a standalone server was
//! refused (`503`) before the next one asked again. The backend's client
//! reports each server-description change to
//! [`write_topology_event_handler`], which forgets the answer when a server's
//! type changes to one without transactions.
//!
//! It is the *server* type that changes, not the topology type: with
//! `directConnection=true` the topology type is `Single` for both a replica set
//! and a standalone server, so `TopologyDescriptionChanged` alone would not
//! show the change.
//!
//! A restart is not reported as a direct `RsPrimary` -> `Standalone` change.
//! The driver first marks the server `Unknown` (the connection dropped, or a
//! check failed) and learns the new type at the next check. A write that starts
//! in between reads the memo before the driver sees `Standalone`, so a change
//! to `Unknown` counts as well (see [`type_change_forgets`]).
//!
//! The handler runs on the driver's own event task, shortly after the change
//! and not before the write that causes it: a write that read the kept answer
//! first still learns of the change from its refused transaction, its failed
//! transaction start or its missing server.

use std::sync::Arc;

use mongodb::ServerType;
use mongodb::event::EventHandler;
use mongodb::event::sdam::SdamEvent;

use super::storage::WriteTopologyMemo;

/// Whether a server of this type answers `hello` in a way the write path reads
/// as transactional: `write_topology_from_hello` (in `storage.rs`) reads
/// `setName` (every replica-set member that has a configuration) and
/// `msg: "isdbgrid"` (a mongos) as transactional.
///
/// `Standalone`, `RsGhost` (a member without a configuration, so no `setName`),
/// `Unknown` (no usable answer) and any type a later driver adds are not.
fn has_transactions(server_type: ServerType) -> bool {
    matches!(
        server_type,
        ServerType::RsPrimary
            | ServerType::RsSecondary
            | ServerType::RsArbiter
            | ServerType::RsOther
            | ServerType::Mongos
            | ServerType::LoadBalancer
    )
}

/// Whether a server whose type changes from `previous` to `new` forgets a kept
/// transactional answer: it does when the type changes to one without
/// transactions ([`has_transactions`]).
///
/// So a replica-set or mongos member that becomes `Standalone`, `Unknown` or
/// `RsGhost`, and an `Unknown` server that turns out `Standalone`, forget. The
/// reverse needs nothing: a standalone answer is never kept, so the next write
/// asks anyway. A change between types with transactions (a stepdown, a member
/// becoming `RsOther`, a server found by discovery) leaves the answer as it is,
/// and so does `Unknown` -> `Unknown`: a server that keeps failing its checks
/// changes its description on each failure, but not its type, and the unit
/// tests of `storage.rs` that run against no server at all rely on it.
pub(super) fn type_change_forgets(previous: ServerType, new: ServerType) -> bool {
    previous != new && !has_transactions(new)
}

/// Handles one server-type change `server` reports: forgets the kept
/// transactional answer of `memo` if [`type_change_forgets`] says so, and
/// returns whether there was one to forget.
///
/// It runs on the driver's event task, so it only touches the memo's atomic and
/// logs: a panic here would end the task and silently stop later events.
pub(super) fn note_server_type_change(
    memo: &WriteTopologyMemo,
    server: &dyn std::fmt::Display,
    previous: ServerType,
    new: ServerType,
) -> bool {
    if !type_change_forgets(previous, new) {
        return false;
    }
    let forgot = memo.forget_transactional_answer();
    if forgot {
        tracing::info!(
            server = %server,
            previous_type = %previous,
            new_type = %new,
            kept_answer_forgotten = forgot,
            "the MongoDB driver reports server {server} as {new} (was {previous}): the kept \
             transactional `hello` answer is forgotten, so the next write asks the server's \
             topology again"
        );
    } else {
        tracing::debug!(
            server = %server,
            previous_type = %previous,
            new_type = %new,
            kept_answer_forgotten = forgot,
            "the MongoDB driver reports server {server} as {new} (was {previous}); no \
             transactional `hello` answer was kept"
        );
    }
    forgot
}

/// Builds the handler a backend registers on its client's topology events: a
/// `ServerDescriptionChanged` event runs [`note_server_type_change`] on `memo`.
/// Other SDAM events are ignored.
///
/// The handler holds `memo` with a strong `Arc`: the client holds the handler
/// and the memo holds nothing, so there is no cycle.
pub(super) fn write_topology_event_handler(
    memo: Arc<WriteTopologyMemo>,
) -> EventHandler<SdamEvent> {
    EventHandler::callback(move |event| {
        if let SdamEvent::ServerDescriptionChanged(change) = event {
            note_server_type_change(
                &memo,
                &change.address,
                change.previous_description.server_type(),
                change.new_description.server_type(),
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::mongodb::storage::WriteTopology;
    use ServerType::{
        LoadBalancer, Mongos, RsArbiter, RsGhost, RsOther, RsPrimary, RsSecondary, Standalone,
        Unknown,
    };

    #[test]
    fn a_change_to_a_type_without_transactions_forgets() {
        for (previous, new) in [
            (RsPrimary, Standalone),
            (RsSecondary, Standalone),
            (Mongos, Standalone),
            (LoadBalancer, Standalone),
            (RsPrimary, Unknown),
            (RsSecondary, Unknown),
            (Mongos, Unknown),
            (Unknown, Standalone),
            (RsPrimary, RsGhost),
            (Unknown, RsGhost),
            (Standalone, Unknown),
        ] {
            assert!(
                type_change_forgets(previous, new),
                "{previous} -> {new} must forget the kept answer"
            );
        }
    }

    #[test]
    fn a_change_between_types_with_transactions_or_to_the_same_type_does_not() {
        for (previous, new) in [
            (RsPrimary, RsSecondary),
            (RsSecondary, RsPrimary),
            (RsPrimary, RsOther),
            (RsSecondary, RsArbiter),
            (Unknown, RsPrimary),
            (Unknown, Mongos),
            (Unknown, LoadBalancer),
            (Standalone, RsPrimary),
            (Standalone, Mongos),
            (Unknown, Unknown),
            (Standalone, Standalone),
            (RsPrimary, RsPrimary),
        ] {
            assert!(
                !type_change_forgets(previous, new),
                "{previous} -> {new} must leave the kept answer as it is"
            );
        }
    }

    #[test]
    fn the_kept_answer_is_forgotten_once_and_a_standalone_one_is_left_as_it_is() {
        let memo = WriteTopologyMemo::default();
        memo.record(WriteTopology::Transactional);
        let server = "localhost:27017";

        assert!(
            !note_server_type_change(&memo, &server, RsPrimary, RsSecondary),
            "a stepdown is not a change to a type without transactions"
        );
        assert!(memo.transactional_answer_kept());

        assert!(
            note_server_type_change(&memo, &server, RsPrimary, Unknown),
            "the server became unreachable: the kept answer is forgotten"
        );
        assert!(!memo.transactional_answer_kept());

        assert!(
            !note_server_type_change(&memo, &server, Unknown, Standalone),
            "nothing left to forget"
        );

        memo.record(WriteTopology::Standalone);
        assert!(!note_server_type_change(
            &memo, &server, Standalone, Unknown
        ));
        assert_eq!(
            memo.record(WriteTopology::Standalone),
            Some(WriteTopology::Standalone),
            "the standalone answer is still remembered, so its warning is not logged again"
        );

        let fresh = WriteTopologyMemo::default();
        assert!(!note_server_type_change(
            &fresh, &server, Unknown, Standalone
        ));
        assert_eq!(
            fresh.record(WriteTopology::Standalone),
            None,
            "a memo that had no answer still has none"
        );
    }
}
