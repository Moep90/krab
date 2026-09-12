//! `kapitan inventory explain`: the provenance trail of one value.

use kapitan_inventory::error::{Error, Result};
use kapitan_inventory::merge::{MergeEvent, get};
use kapitan_inventory::source::Location;
use kapitan_inventory::{Inventory, KeyPath, RenderedTarget, Value};
use serde::Serialize;

#[derive(Serialize)]
struct Explanation<'a> {
    target: &'a str,
    path: String,
    value: &'a Value,
    r#type: &'static str,
    origin: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_from: Option<ResolvedFrom>,
    history: Vec<HistoryEntry>,
    children_history: usize,
}

#[derive(Serialize)]
struct ResolvedFrom {
    expr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_origin: Option<Location>,
}

#[derive(Serialize)]
struct HistoryEntry {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_value: Option<Value>,
    old_type: Option<&'static str>,
    new_location: Option<Location>,
    new_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

pub fn explain(inv: &Inventory, target: &RenderedTarget, path: &str, json: bool) -> Result<()> {
    let key = KeyPath::parse(path.strip_prefix("parameters.").unwrap_or(path));
    let node = get(&target.parameters, &key).ok_or_else(|| {
        Error::new("inventory::path_not_found", format!("no value at `{key}` in target `{}`", target.name))
            .with_help("paths are relative to `parameters`, e.g. cluster.name or kapitan.compile[0].name")
    })?;
    let loc = |o| Location::from_origin(&inv.sources, o);
    let mut history = Vec::new();
    for ev in target.provenance.merge_events(&key, false) {
        history.push(match ev {
            MergeEvent::Override { old, new, old_value, old_type, new_type, .. } => HistoryEntry {
                kind: "override",
                old_location: loc(*old),
                old_value: old_value.clone(),
                old_type: Some(old_type),
                new_location: loc(*new),
                new_type: Some(new_type),
                detail: None,
            },
            MergeEvent::ListAppend { index, origin, .. } => HistoryEntry {
                kind: "list_append",
                old_location: None,
                old_value: None,
                old_type: None,
                new_location: loc(*origin),
                new_type: None,
                detail: Some(format!("item [{index}] appended")),
            },
            MergeEvent::Dereference { expr, origin, .. } => HistoryEntry {
                kind: "dereference",
                old_location: loc(*origin),
                old_value: None,
                old_type: None,
                new_location: None,
                new_type: None,
                detail: Some(format!("placeholder {expr} expanded so a mapping could be merged into it")),
            },
        });
    }
    let children_history = target.provenance.merge_events(&key, true).count() - history.len();
    let resolved_from = target.provenance.resolution_of(&key).map(|r| ResolvedFrom {
        expr: r.expr.clone(),
        source_path: r.source.as_ref().map(|p| p.to_string()),
        source_origin: r.source.as_ref().and_then(|p| get(&target.parameters, p)).and_then(|n| loc(n.origin)),
    });
    let explanation = Explanation {
        target: &target.name,
        path: key.to_string(),
        value: &node.value,
        r#type: node.value.type_name(),
        origin: loc(node.origin),
        resolved_from,
        history,
        children_history,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&explanation).unwrap());
        return Ok(());
    }
    let short = |v: &Value| {
        let s = match v {
            Value::Map(m) => format!("{{…{} keys}}", m.len()),
            Value::List(l) => format!("[…{} items]", l.len()),
            other => other.py_repr(),
        };
        if s.chars().count() > 70 { format!("{}…", s.chars().take(69).collect::<String>()) } else { s }
    };
    println!("{}  ({})", explanation.path, explanation.r#type);
    println!("  value: {}", short(&node.value));
    match &explanation.origin {
        Some(l) => println!("  written at {l}"),
        None => println!("  written by kapitan (synthetic)"),
    }
    if let Some(r) = &explanation.resolved_from {
        print!("  resolved from {}", r.expr);
        if let Some(p) = &r.source_path {
            print!(" -> {p}");
        }
        if let Some(l) = &r.source_origin {
            print!(" ({l})");
        }
        println!();
    }
    if explanation.history.is_empty() {
        println!("  never overridden");
    } else {
        println!("  history (oldest first):");
        for h in &explanation.history {
            match h.kind {
                "override" => {
                    let old = h.old_value.as_ref().map(short).unwrap_or_else(|| format!("<{}>", h.old_type.unwrap_or("?")));
                    println!(
                        "    {} overrode {} from {}",
                        h.new_location.as_ref().map(|l| l.to_string()).unwrap_or_else(|| "<synthetic>".into()),
                        old,
                        h.old_location.as_ref().map(|l| l.to_string()).unwrap_or_else(|| "<synthetic>".into())
                    );
                }
                _ => println!(
                    "    {} {}",
                    h.new_location.as_ref().or(h.old_location.as_ref()).map(|l| l.to_string()).unwrap_or_default(),
                    h.detail.clone().unwrap_or_default()
                ),
            }
        }
    }
    if explanation.children_history > 0 {
        println!("  ({} more events under this path; explain a child path to see them)", explanation.children_history);
    }
    Ok(())
}
