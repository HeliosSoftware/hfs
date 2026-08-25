//! SQL-on-FHIR operation handlers.
//!
//! SQL on FHIR 3.0.0-ballot defines two data operations, both invoked at the
//! system level and both acting on a *subject* — a ViewDefinition, a SQLQuery
//! Library or a SQLView Library:
//!
//! - [`run`] — `$sql-run`, synchronous, one subject, rows in the response
//! - [`export`] — `$sql-export`, asynchronous, many subjects, one job and one manifest
//!
//! [`subject`] owns the naming and resolution shared by both.

pub mod capability;
pub mod export;
pub(crate) mod graph;
pub(crate) mod references;
pub mod run;
pub mod sqlquery;
pub mod subject;
pub(crate) mod view_sources;

pub use capability::sof_operation_definition_handler;
pub use export::{
    cancel_export_handler, download_export_file_handler, get_export_result_handler,
    get_export_status_handler, sql_export_handler,
};
pub use run::sql_run_handler;
