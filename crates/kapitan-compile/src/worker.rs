//! Python worker processes speaking newline-delimited JSON; the
//! implementation lives in the inventory crate, shared with the Python
//! resolvers.

pub use kapitan_inventory::python::{Worker, WorkerError};
