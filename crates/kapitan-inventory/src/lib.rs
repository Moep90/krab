//! Kapitan inventory engine.
//!
//! Loads an inventory (`targets/` + `classes/`), resolves class inheritance,
//! merges parameters with OmegaConf-compatible semantics, evaluates `${...}`
//! interpolations through a pluggable resolver registry and tracks where every
//! value came from.

pub mod error;
pub mod path;
pub mod source;
pub mod value;
pub mod yaml;

pub use error::{Diagnostic, Error, Result, Severity};
pub use path::{Key, KeyPath};
pub use source::{Location, Origin, SourceId, Sources};
pub use value::{Map, Node, Value};
pub mod classfile;
pub mod emit;
pub mod interp;
pub mod inventory;
pub mod merge;
pub mod model;
pub mod pyfmt;
pub mod resolvers;

pub use inventory::{
    ClassUsage, Inventory, InventoryConfig, RenderReport, RenderedTarget, TargetSpec,
};
pub use resolvers::Registry;
pub mod dotkapitan;
pub mod explain;
