//! Pretty (miette) and JSON rendering of diagnostics.

use std::collections::HashMap;

use krab_inventory::error::{Diagnostic, Severity};
use miette::{LabeledSpan, NamedSource, Report, SourceSpan};

/// A diagnostic wrapped for miette: the first labelled location gets a
/// source snippet, the others are listed in the help text.
#[derive(Debug)]
struct Rendered {
    message: String,
    code: String,
    help: Option<String>,
    source: Option<NamedSource<String>>,
    labels: Vec<LabeledSpan>,
}

impl std::fmt::Display for Rendered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Rendered {}

impl miette::Diagnostic for Rendered {
    fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        Some(Box::new(&self.code))
    }

    fn help<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.help
            .as_ref()
            .map(|h| Box::new(h) as Box<dyn std::fmt::Display>)
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.source.as_ref().map(|s| s as &dyn miette::SourceCode)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        if self.labels.is_empty() {
            None
        } else {
            Some(Box::new(self.labels.iter().cloned()))
        }
    }

    fn severity(&self) -> Option<miette::Severity> {
        None
    }
}

fn offset_of(text: &str, line: u32, col: u32) -> usize {
    let mut offset = 0;
    for (i, l) in text.split_inclusive('\n').enumerate() {
        if i + 1 == line as usize {
            let col = (col.max(1) - 1) as usize;
            let in_line = l
                .char_indices()
                .nth(col)
                .map(|(b, _)| b)
                .unwrap_or(l.trim_end_matches('\n').len());
            return offset + in_line;
        }
        offset += l.len();
    }
    text.len()
}

fn span_len(text: &str, offset: usize) -> usize {
    // Highlight up to the end of the YAML scalar on that line (until a comment or line end).
    let rest = &text[offset..];
    let end = rest.find('\n').unwrap_or(rest.len());
    let line = &rest[..end];
    let cut = line.find(" #").unwrap_or(line.len());
    cut.max(1)
}

pub fn to_report(d: &Diagnostic) -> Report {
    let mut message = d.message.clone();
    if let Some(t) = &d.target {
        message = format!("[{t}] {message}");
    }
    if let Some(p) = &d.path {
        message = format!("{message}\n  at parameters.{p}");
    }
    let mut source = None;
    let mut labels = Vec::new();
    let mut extra: Vec<String> = Vec::new();
    let mut texts: HashMap<std::path::PathBuf, Option<String>> = HashMap::new();
    let mut primary_file: Option<std::path::PathBuf> = None;
    for label in &d.labels {
        let Some(loc) = &label.location else { continue };
        let text = texts
            .entry(loc.file.clone())
            .or_insert_with(|| std::fs::read_to_string(&loc.file).ok());
        match (&primary_file, text) {
            (None, Some(text)) => {
                let offset = offset_of(text, loc.line, loc.col);
                let len = span_len(text, offset);
                labels.push(LabeledSpan::new_with_span(
                    Some(label.text.clone()),
                    SourceSpan::new(offset.into(), len),
                ));
                source = Some(NamedSource::new(loc.file.to_string_lossy(), text.clone()));
                primary_file = Some(loc.file.clone());
            }
            (Some(pf), Some(text)) if pf == &loc.file => {
                let offset = offset_of(text, loc.line, loc.col);
                let len = span_len(text, offset);
                labels.push(LabeledSpan::new_with_span(
                    Some(label.text.clone()),
                    SourceSpan::new(offset.into(), len),
                ));
            }
            _ => extra.push(format!("{}: {}", loc, label.text)),
        }
    }
    let mut help = d.help.clone();
    if !extra.is_empty() {
        let related = format!("see also:\n  {}", extra.join("\n  "));
        help = Some(match help {
            Some(h) => format!("{h}\n{related}"),
            None => related,
        });
    }
    Report::new(Rendered {
        message,
        code: d.code.to_string(),
        help,
        source,
        labels,
    })
}

pub fn print_diagnostic(d: &Diagnostic, json: bool) {
    if json {
        println!("{}", serde_json::to_string(d).unwrap());
        return;
    }
    let report = to_report(d);
    let prefix = match d.severity {
        Severity::Error => "",
        Severity::Warning => "warning: ",
    };
    eprintln!("{prefix}{report:?}");
}
