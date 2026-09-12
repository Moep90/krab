//! Incremental compilation of kapitan targets.
//!
//! The Rust side renders the inventory, decides per target whether anything
//! it was built from changed (inventory document, every file and directory
//! the previous compile read, other targets' inventories it consulted,
//! compiler version, or the compiled output itself), and drives a pool of
//! Python workers that run kapitan's own input types for the stale ones.

pub mod digest;
pub mod engine;
pub mod manifest;
pub mod plan;
pub mod python;
pub mod worker;

pub use engine::{CompileOptions, DocSource, Event, Outcome, Report, Selection, Status, compile};
pub use manifest::{Manifest, TargetRecord};
pub use plan::TargetPlan;
pub use python::PythonCmd;
