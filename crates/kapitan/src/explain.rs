//! Text rendering of an `Explanation` (the JSON form is the struct itself).

use kapitan_inventory::Value;
use kapitan_inventory::explain::{Explanation, HistoryEntry};
use kapitan_inventory::source::Location;

fn short(v: &Value) -> String {
    let s = match v {
        Value::Map(m) => format!("{{…{} keys}}", m.len()),
        Value::List(l) => format!("[…{} items]", l.len()),
        other => other.py_repr(),
    };
    if s.chars().count() > 70 { format!("{}…", s.chars().take(69).collect::<String>()) } else { s }
}

fn loc(l: &Option<Location>) -> String {
    l.as_ref().map(|l| l.to_string()).unwrap_or_else(|| "<kapitan>".into())
}

pub fn print_explanation(e: &Explanation) {
    println!("{}  ({})", e.path, e.r#type);
    println!("  value: {}", short(&e.value));
    match &e.origin {
        Some(l) => println!("  written at {l}"),
        None => println!("  written by kapitan (synthetic)"),
    }
    if let Some(r) = &e.resolved_from {
        print!("  resolved from {}", r.expr);
        if let Some(p) = &r.source_path {
            print!(" -> {p}");
        }
        if let Some(l) = &r.source_origin {
            print!(" ({l})");
        }
        println!();
    }
    if e.history.is_empty() {
        println!("  never overridden");
    } else {
        println!("  history (oldest first):");
        for h in &e.history {
            match h {
                HistoryEntry::Override { old_location, old_value, old_type, new_location, .. } => {
                    let old = old_value.as_ref().map(short).unwrap_or_else(|| format!("<{old_type}>"));
                    println!("    {} overrode {} from {}", loc(new_location), old, loc(old_location));
                }
                HistoryEntry::ListAppend { index, location } => println!("    {} appended item [{index}]", loc(location)),
                HistoryEntry::Dereference { expr, location } => {
                    println!("    {} placeholder {expr} expanded so a mapping could be merged into it", loc(location))
                }
            }
        }
    }
    if e.children_events > 0 {
        println!("  ({} more events under this path; explain a child path to see them)", e.children_events);
    }
}
