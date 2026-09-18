//! `${...}` interpolation: grammar, parsing and evaluation.

pub mod ast;
pub mod eval;
pub mod parse;

pub use ast::{Element, Interp, KeySeg, NamePart, Prim, Text, TextPart};
pub use eval::{Evaluator, ResolveEvent, Resolved};
pub use parse::{ParseError, parse_element, parse_text};
