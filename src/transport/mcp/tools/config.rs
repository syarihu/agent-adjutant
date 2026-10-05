use crate::kernel::config;
use serde_json::{Value, json};
use std::path::Path;

use super::resolve_repo;

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let info = resolve_repo(args)?;
    let resolved = config::resolve_config(&info.nwo)?;
    Ok(json!({
        "repo": info.nwo,
        "main": info.main,
        "hub": info.hub,
        "hubName": info.hub_name,
        // The board running for this hub, or null, and whether the resident server
        // serves it. Read from records, so one started by hand with `adj serve` is
        // found as well as the hub's own.
        "board": crate::board::located(
            &crate::registry::state_root(Some(Path::new(&info.main))),
            &info,
        ),
        "registered": resolved.registered,
        "configPath": resolved.config_path,
        "warnings": resolved.warnings,
        "settings": resolved.settings,
        "config": resolved.config,
    }))
}
