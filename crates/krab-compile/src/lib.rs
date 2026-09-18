//! Incremental compilation of kapitan targets.
//!
//! The Rust side renders the inventory, decides per target whether anything
//! it was built from changed (inventory document, every file and directory
//! the previous compile read, other targets' inventories it consulted,
//! compiler version, or the compiled output itself), and drives a pool of
//! Python workers that run kapitan's own input types for the stale ones.

pub mod digest;
pub mod docs;
pub mod engine;
pub mod fetch;
pub mod inputs;
pub mod manifest;
pub mod native;
pub mod oci;
pub mod output;
pub mod plan;
pub mod pyenv;
pub mod python;
pub mod refs;
pub mod worker;

pub use docs::{DocProvider, MapDocs, SharedDocs};
pub use engine::{
    Backend, CompileOptions, DocSource, Event, Outcome, Report, Selection, Status, compile,
};
pub use fetch::{Dependency, FetchOutcome, FetchStatus};
pub use manifest::{Manifest, TargetRecord};
pub use native::NativeOptions;
pub use plan::TargetPlan;
pub use pyenv::PythonEnv;
pub use python::{PythonCmd, PythonNeeds, PythonProbe};
