use crate::kernel::identity;
use crate::registry;
use serde_json::{Value, json};
use std::path::PathBuf;

pub(super) mod config;
pub(super) mod gate_close;
pub(super) mod gate_open;
pub(super) mod hub_status;
pub(super) mod outbox;
pub(super) mod pending;
pub(super) mod refresh;
pub(super) mod send;
pub(super) mod skill;
pub(super) mod tell;

pub(super) fn cwd_param(args: &Value) -> Option<PathBuf> {
    args["cwd"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(crate::infra::paths::expand_home)
}

pub(super) fn resolve_repo(args: &Value) -> Result<identity::RepoInfo, String> {
    let cwd = cwd_param(args);
    // `cwd` and not the server's own directory, for the same reason the repository is
    // resolved from it: a worker's answer is written in the worktree it is standing in, and
    // this server is started once and then asked about whichever checkout the session is
    // sitting in.
    let hub = registry::hub_id(args["hub"].as_str(), cwd.as_deref())?;
    identity::resolve_in(
        cwd.as_deref(),
        args["repo"].as_str().filter(|s| !s.is_empty()),
        hub.as_deref(),
    )
}

/// One MCP tool: the one place its name, advertised definition and handler are tied together.
pub(super) struct Tool {
    pub(super) name: &'static str,
    pub(super) definition: fn() -> Value,
    pub(super) call: fn(&Value) -> Result<Value, String>,
}

/// Every tool, in the order `tools/list` advertises them.
pub(super) const TOOLS: &[Tool] = &[
    Tool {
        name: "adjutant_config",
        definition: config::definition,
        call: config::call,
    },
    Tool {
        name: "adjutant_hub_status",
        definition: hub_status::definition,
        call: hub_status::call,
    },
    Tool {
        name: "adjutant_send",
        definition: send::definition,
        call: send::call,
    },
    Tool {
        name: "adjutant_pending",
        definition: pending::definition,
        call: pending::call,
    },
    Tool {
        name: "adjutant_tell",
        definition: tell::definition,
        call: tell::call,
    },
    Tool {
        name: "adjutant_outbox",
        definition: outbox::definition,
        call: outbox::call,
    },
    Tool {
        name: "adjutant_gate_open",
        definition: gate_open::definition,
        call: gate_open::call,
    },
    Tool {
        name: "adjutant_gate_close",
        definition: gate_close::definition,
        call: gate_close::call,
    },
    Tool {
        name: "adjutant_refresh",
        definition: refresh::definition,
        call: refresh::call,
    },
    Tool {
        name: "adjutant_skill",
        definition: skill::definition,
        call: skill::call,
    },
];

fn repo_property() -> Value {
    json!({
        "type": "string",
        "description": "owner/name. Defaults to the repository of `cwd`, taken from its origin remote.",
    })
}

/// Named `hub` rather than `hubName`: what goes in here is the identifier a person typed
/// after `--hub`, not the `adjutant-…` session name the tools answer with.
fn hub_property() -> Value {
    json!({
        "type": "string",
        "description": "Which hub of the repository, when it is not the repository's own one. Leave it out unless you were told otherwise: a hub already knows its own, and a worker's is read from the worktree it is in.",
    })
}

fn cwd_property() -> Value {
    json!({
        "type": "string",
        "description": "Directory to answer for. Defaults to the server's own working directory; pass the worktree you are in if that is somewhere else.",
    })
}
