use super::Launch;
use crate::lifecycle::agent_command;
use crate::registry::{self, Context};
use std::os::unix::process::CommandExt;

/// Replace this process with the hub's agent.
///
/// The process registers itself and then *replaces* itself with the agent, so the recorded
/// PID belongs to the live agent rather than to a launcher that has already exited. Returns
/// only if the exec failed, after undoing the claim, with what to tell the person.
pub fn exec_launch(ctx: &Context, launch: &Launch) -> String {
    // `exec` keeps the PID, which is the whole point: the record written a line ago has to
    // name the process a worker will later check for.
    // Removed and then set on the line itself, so the only value the agent — and so its MCP
    // server — can see is this hub's own, never one inherited from whatever started this.
    let error = agent_command(&launch.command).exec();
    // Only reachable if exec failed — otherwise this process no longer exists.
    let _ = registry::unregister_hub(&ctx.state, &ctx.repo.slug);
    format!("cannot start the hub: {error}")
}
