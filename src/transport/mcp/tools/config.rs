use crate::kernel::config;
use serde_json::{Value, json};
use std::path::Path;

use super::{cwd_property, hub_property, repo_property, resolve_repo};

pub(super) fn definition() -> Value {
    json!({
        "name": "adjutant_config",
        "description": "The resolved configuration for a repository: task sources (always an array, flat shorthand already expanded), defaults already merged, and the machine settings (terminal, agent runner, notification, editor, the language the person reads, whether to collect the dashboard at startup). Also reports warnings about the config rather than failing on it. Call this once at startup instead of reading the config file.",
        "inputSchema": {
            "type": "object",
            "properties": { "repo": repo_property(), "hub": hub_property(), "cwd": cwd_property() },
        },
    })
}

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
