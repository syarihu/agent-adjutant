use crate::kernel::identity;
use crate::registry;
use serde_json::{Value, json};
use std::path::PathBuf;

pub(super) mod config;
pub(super) mod gate_answer;
pub(super) mod gate_close;
pub(super) mod gate_open;
pub(super) mod gate_read;
pub(super) mod hub_status;
pub(super) mod outbox;
pub(super) mod pending;
pub(super) mod refresh;
pub(super) mod request;
pub(super) mod reviews;
pub(super) mod send;
pub(super) mod skill;
pub(super) mod tell;
pub(super) mod work;

pub(super) fn cwd_param(args: &Value) -> Option<PathBuf> {
    args["cwd"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(crate::infra::paths::expand_home)
}

/// A string argument, with blank the same as absent.
pub(super) fn text_param<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

/// The context of the board a tool is told by its `slug`, for the tools that act on a board from
/// anywhere (answer a gate, pass a request). The slug is checked before it becomes a path.
pub(super) fn board_context(args: &Value) -> Result<registry::Context, String> {
    let board = text_param(args, "board").ok_or("pass `board`: the `slug` from adjutant_work")?;
    registry::context_of_board(&registry::state_root(None), board)
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
    Tool {
        name: "adjutant_work",
        definition: work::definition,
        call: work::call,
    },
    Tool {
        name: "adjutant_gate_read",
        definition: gate_read::definition,
        call: gate_read::call,
    },
    Tool {
        name: "adjutant_reviews",
        definition: reviews::definition,
        call: reviews::call,
    },
    Tool {
        name: "adjutant_request",
        definition: request::definition,
        call: request::call,
    },
    Tool {
        name: "adjutant_gate_answer",
        definition: gate_answer::definition,
        call: gate_answer::call,
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
