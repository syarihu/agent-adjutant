use super::Launch;
use crate::lifecycle::agent_command;
use crate::registry;
use std::os::unix::process::CommandExt;

/// Replace this process with the worker's agent.
///
/// Returns only if the exec failed, after unregistering the worker, with what to tell the
/// person.
pub fn exec_launch(launch: &Launch) -> String {
    // A worker is not a hub. A tab opened by a spawn command that passes its environment on
    // would otherwise hand the hub's session to this agent's MCP server, which would then
    // keep saying the hub is alive for as long as the worker runs — and the hub's board
    // marker, which would have it try to serve the hub's board.
    //
    // Nor is it whichever hub opened the tab. `ADJUTANT_HUB` outranks the record
    // `register_launch` wrote, so an inherited one would send every report to the hub that
    // dispatched the tab rather than the one this worker registered under; the line carries
    // the right one.
    let error = agent_command(&launch.command).exec();
    // Only reachable if exec failed — otherwise this process no longer exists.
    let _ = registry::unregister_worker(&launch.worktree);
    format!("cannot start the worker: {error}")
}
