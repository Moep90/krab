//! Everything the language server knows comes from the inventory daemon
//! (started on demand) plus a render-free local `Inventory` for class file
//! resolution.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kapitan_inventory::explain::Explanation;
use kapitan_inventory::{ClassUsage, Inventory, InventoryConfig, Registry};
use kapitan_server::protocol::*;
use kapitan_server::{Client, ClientError, Connector};
use parking_lot::Mutex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

pub struct Backend {
    pub connector: Connector,
    client: Mutex<Option<Client>>,
    pub inv: Inventory,
}

impl Backend {
    pub fn new(
        connector: Connector,
        inventory_root: PathBuf,
        compose_target_name: bool,
    ) -> Backend {
        let mut cfg = InventoryConfig::new(inventory_root);
        cfg.compose_target_name = compose_target_name;
        Backend {
            connector,
            client: Mutex::new(None),
            inv: Inventory::new(cfg, Arc::new(Registry::new())),
        }
    }

    /// Call the daemon, reconnecting (or starting it) once when the
    /// connection is gone.
    pub fn call<P: Serialize + Clone, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, String> {
        let mut guard = self.client.lock();
        for attempt in 0..2 {
            if guard.is_none() {
                *guard = Some(
                    self.connector
                        .connect_or_spawn()
                        .map_err(|e| e.to_string())?,
                );
            }
            match guard.as_mut().unwrap().call::<P, R>(method, params.clone()) {
                Ok(r) => return Ok(r),
                Err(ClientError::Rpc(e)) => return Err(e.message),
                Err(e) => {
                    *guard = None;
                    if attempt == 1 {
                        return Err(e.to_string());
                    }
                }
            }
        }
        unreachable!()
    }

    /// A dedicated connection for long polls, so they never block other requests.
    pub fn wait_client(&self) -> Result<Client, String> {
        self.connector.connect_or_spawn().map_err(|e| e.to_string())
    }

    /// Targets rendered from `file` (a class or target file).
    pub fn targets_for_file(&self, file: &Path) -> Vec<String> {
        self.call(
            "inventory.deps",
            DepsParams {
                files: vec![file.to_path_buf()],
            },
        )
        .unwrap_or_default()
    }

    pub fn explain(&self, target: &str, path: &str) -> Option<Explanation> {
        self.call(
            "inventory.explain",
            ExplainParams {
                target: target.to_string(),
                path: path.to_string(),
            },
        )
        .ok()
    }

    pub fn target_value(&self, target: &str, path: Option<&str>) -> Option<Json> {
        self.call::<_, TargetResult>(
            "inventory.target",
            TargetParams {
                name: target.to_string(),
                path: path.map(str::to_string),
            },
        )
        .ok()
        .map(|r| r.document)
    }

    pub fn targets(&self) -> Vec<TargetSummary> {
        self.call::<_, TargetsResult>("inventory.targets", Json::Null)
            .map(|r| r.targets)
            .unwrap_or_default()
    }

    pub fn diagnostics(&self) -> Option<DiagnosticsResult> {
        self.call("inventory.diagnostics", Json::Null).ok()
    }

    pub fn class_usage(&self) -> Vec<ClassUsage> {
        self.call("inventory.class_usage", Json::Null)
            .unwrap_or_default()
    }

    /// Resolve a class name as written in `from_file`'s `classes:` list.
    pub fn resolve_class(&self, name: &str, from_file: &Path) -> Option<PathBuf> {
        self.inv.resolve_class_file(name, from_file).found
    }
}
