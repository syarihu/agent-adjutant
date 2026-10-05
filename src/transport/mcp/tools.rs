use crate::kernel::identity;
use crate::registry;
use serde_json::Value;
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
