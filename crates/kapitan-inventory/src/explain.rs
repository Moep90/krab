//! Provenance of one value: where it was written, what it overrode, which
//! interpolation produced it.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::inventory::{Inventory, RenderedTarget};
use crate::merge::{MergeEvent, get};
use crate::path::KeyPath;
use crate::source::Location;
use crate::value::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Explanation {
    pub target: String,
    /// Path relative to `parameters`, OmegaConf style (`a.b[0].c`).
    pub path: String,
    pub value: Value,
    pub r#type: String,
    /// Where the current value was written; `None` for values kapitan made up.
    pub origin: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_from: Option<ResolvedFrom>,
    /// Oldest first.
    pub history: Vec<HistoryEntry>,
    /// Number of merge events under this path that are not shown.
    pub children_events: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedFrom {
    pub expr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_origin: Option<Location>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryEntry {
    Override {
        old_location: Option<Location>,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_value: Option<Value>,
        old_type: String,
        new_location: Option<Location>,
        new_type: String,
    },
    ListAppend { index: usize, location: Option<Location> },
    Dereference { expr: String, location: Option<Location> },
}

pub fn explain(inv: &Inventory, target: &RenderedTarget, path: &str) -> Result<Explanation> {
    let key = KeyPath::parse(path.strip_prefix("parameters.").unwrap_or(path));
    let node = get(&target.parameters, &key).ok_or_else(|| {
        Error::new("inventory::path_not_found", format!("no value at `{key}` in target `{}`", target.name))
            .with_target(&target.name)
            .with_help("paths are relative to `parameters`, e.g. cluster.name or kapitan.compile[0].name")
    })?;
    let loc = |o| Location::from_origin(&inv.sources, o);
    let history: Vec<HistoryEntry> = target
        .provenance
        .merge_events(&key, false)
        .map(|ev| match ev {
            MergeEvent::Override { old, new, old_value, old_type, new_type, .. } => HistoryEntry::Override {
                old_location: loc(*old),
                old_value: old_value.clone(),
                old_type: old_type.to_string(),
                new_location: loc(*new),
                new_type: new_type.to_string(),
            },
            MergeEvent::ListAppend { index, origin, .. } => HistoryEntry::ListAppend { index: *index, location: loc(*origin) },
            MergeEvent::Dereference { expr, origin, .. } => {
                HistoryEntry::Dereference { expr: expr.clone(), location: loc(*origin) }
            }
        })
        .collect();
    let children_events = target.provenance.merge_events(&key, true).count() - history.len();
    let resolved_from = target.provenance.resolution_of(&key).map(|r| ResolvedFrom {
        expr: r.expr.clone(),
        source_path: r.source.as_ref().map(|p| p.to_string()),
        source_origin: r.source.as_ref().and_then(|p| get(&target.parameters, p)).and_then(|n| loc(n.origin)),
    });
    Ok(Explanation {
        target: target.name.clone(),
        path: key.to_string(),
        value: node.value.clone(),
        r#type: node.value.type_name().to_string(),
        origin: loc(node.origin),
        resolved_from,
        history,
        children_events,
    })
}
