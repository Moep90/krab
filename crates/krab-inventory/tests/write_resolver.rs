//! `write` into a destination whose parents do not exist yet: the reference
//! crashes ("dictionary changed size during iteration"); krab creates it.

use krab_inventory::{Inventory, InventoryConfig};

#[test]
fn write_creates_a_missing_destination() {
    let dir = std::env::temp_dir().join(format!("krab-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("targets")).unwrap();
    std::fs::write(
        dir.join("targets/t.yml"),
        "parameters:\n  src:\n    a: 1\n    b: ${src.a}\n  dump:\n    - \\${write:fresh.x.y,src}\n",
    )
    .unwrap();
    let inv = Inventory::new(
        InventoryConfig::new(dir.clone()),
        std::sync::Arc::new(krab_inventory::resolvers::Registry::with_builtins()),
    );
    let report = inv.render_all().unwrap();
    if let Some(e) = report.errors.first() {
        panic!("{e}");
    }
    let doc = report.targets["t"].to_document().value.to_json();
    assert_eq!(
        doc["parameters"]["fresh"]["x"]["y"],
        serde_json::json!({"a": 1, "b": 1})
    );
    assert_eq!(doc["parameters"]["dump"], serde_json::json!(["DONE"]));
    let _ = std::fs::remove_dir_all(&dir);
}
