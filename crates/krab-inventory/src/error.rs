//! Diagnostics. Every failure is a structured [`Diagnostic`] with a stable
//! code, a message, optional labelled source locations and a help text, so
//! the CLI can render it prettily and IDEs / LLM tooling can consume it as JSON.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::source::{Location, Origin, Sources};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Label {
    #[serde(skip)]
    pub origin: Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable machine readable code, e.g. `inventory::class_not_found`.
    pub code: Cow<'static, str>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Key path in the parameters tree this diagnostic is about, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            code: Cow::Borrowed(code),
            message: message.into(),
            target: None,
            path: None,
            labels: Vec::new(),
            help: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Self::error(code, message)
        }
    }

    pub fn with_label(mut self, origin: Origin, text: impl Into<String>) -> Self {
        if !origin.is_synthetic() {
            self.labels.push(Label {
                origin,
                location: None,
                text: text.into(),
            });
        }
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Fill in file paths for every label (they are only ids until now).
    pub fn resolve(mut self, sources: &Sources) -> Self {
        for label in &mut self.labels {
            label.location = Location::from_origin(sources, label.origin);
        }
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(t) = &self.target {
            write!(f, "[{t}] ")?;
        }
        write!(f, "{}", self.message)?;
        for label in &self.labels {
            if let Some(loc) = &label.location {
                write!(f, "\n  --> {loc}: {}", label.text)?;
            }
        }
        if let Some(h) = &self.help {
            write!(f, "\n  help: {h}")?;
        }
        Ok(())
    }
}

/// The library error type: a boxed diagnostic.
#[derive(Clone, Debug)]
pub struct Error(pub Box<Diagnostic>);

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Error(Box::new(Diagnostic::error(code, message)))
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.0
    }

    pub fn into_diagnostic(self) -> Diagnostic {
        *self.0
    }

    pub fn with_label(mut self, origin: Origin, text: impl Into<String>) -> Self {
        *self.0 =
            std::mem::replace(&mut *self.0, Diagnostic::error("", "")).with_label(origin, text);
        self
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.0.target = Some(target.into());
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.0.path = Some(path.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.0.help = Some(help.into());
        self
    }

    pub fn resolve(self, sources: &Sources) -> Self {
        Error(Box::new(self.0.resolve(sources)))
    }
}

impl From<Diagnostic> for Error {
    fn from(d: Diagnostic) -> Self {
        Error(Box::new(d))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new("io", e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
