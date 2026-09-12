//! Language server for Kapitan inventories, a thin translator over the
//! inventory daemon: diagnostics as files are saved, hover with the resolved
//! value and its provenance, go to definition for class names and `${…}`
//! references, completion of class names and parameter paths.

pub mod backend;
pub mod server;
pub mod yaml_index;

pub use backend::Backend;
pub use server::run_stdio;
