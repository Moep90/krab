//! AST of an OmegaConf interpolation string such as
//! `prefix-${a.b}-${resolver:arg1, ${c}, [1, 2], {k: v}}`.

use std::fmt;

/// A whole string value: literal chunks interleaved with interpolations.
#[derive(Clone, Debug, PartialEq)]
pub struct Text(pub Vec<TextPart>);

#[derive(Clone, Debug, PartialEq)]
pub enum TextPart {
    Lit(String),
    Interp(Interp),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Interp {
    /// `${a.b[0]}`; `dots` leading dots make the path relative (`${.x}`, `${..x}`).
    Node { dots: usize, keys: Vec<KeySeg> },
    /// `${name:arg, arg}`.
    Resolver { name: Vec<NamePart>, args: Vec<Element> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum KeySeg {
    Lit(String),
    Interp(Box<Interp>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NamePart {
    Lit(String),
    Interp(Box<Interp>),
}

/// A resolver argument or list/dict element.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Prim(Prim),
    Quoted(Text),
    List(Vec<Element>),
    Dict(Vec<(Prim, Element)>),
}

/// An unquoted primitive. A single token is typed; several tokens concatenate
/// into a string.
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Interp(Box<Interp>),
    Concat(Vec<TextPart>),
}

impl Text {
    /// The text is exactly one interpolation with nothing around it.
    pub fn single_interp(&self) -> Option<&Interp> {
        match self.0.as_slice() {
            [TextPart::Interp(i)] => Some(i),
            _ => None,
        }
    }
}

impl fmt::Display for Interp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Interp::Node { dots, keys } => {
                write!(f, "${{")?;
                for _ in 0..*dots {
                    write!(f, ".")?;
                }
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ".")?;
                    }
                    match k {
                        KeySeg::Lit(s) => write!(f, "{s}")?,
                        KeySeg::Interp(i) => write!(f, "{i}")?,
                    }
                }
                write!(f, "}}")
            }
            Interp::Resolver { name, args } => {
                write!(f, "${{")?;
                for (i, n) in name.iter().enumerate() {
                    if i > 0 {
                        write!(f, ".")?;
                    }
                    match n {
                        NamePart::Lit(s) => write!(f, "{s}")?,
                        NamePart::Interp(i) => write!(f, "{i}")?,
                    }
                }
                write!(f, ":")?;
                for (i, _) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ",")?;
                    }
                    write!(f, "…")?;
                }
                write!(f, "}}")
            }
        }
    }
}
