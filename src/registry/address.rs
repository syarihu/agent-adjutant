//! Which hub an invocation is addressing.

use super::store::worker_record_path;
use super::*;

/// Which hub this invocation was *told* it is: what the caller passed (`--hub`, or the
/// tool's `hub`), and failing that `ADJUTANT_HUB` — the process was started by a hub, so it
/// is that hub wherever it has since wandered to.
///
/// This is the whole answer for a command that starts or registers a hub rather than
/// addressing one. `adj hub` may be run from inside a worktree, and `adj worker` runs in a
/// tab opened *at* the worktree it is about to register in: both would otherwise read a
/// record that is either somebody else's or the one they are seconds from overwriting. A
/// crashed worker leaves its record behind, so re-dispatching that task under the
/// repository's own hub would file the new worker under the old one — and every report it
/// ever sends goes to a hub that may not even be running.
pub fn hub_id_told(explicit: Option<&str>) -> Option<String> {
    said(explicit).or_else(|| said(std::env::var(HUB_ENV).ok().as_deref()))
}

/// The identifier `agentEnv` hands the agent, when it names one: the last assignment, since
/// that is the one an `env` line leaves standing.
///
/// A command that starts or registers a hub asks this after `hub_id_told` comes back empty.
/// Asked any later, the command would claim one hub's record, inbox and session name while
/// the agent it starts is given another's address — and every report either side sends would
/// go where the other is not reading.
pub fn hub_id_configured(agent_env: &[(String, String)]) -> Option<String> {
    agent_env
        .iter()
        .rev()
        .find(|(key, _)| key == HUB_ENV)
        .and_then(|(_, value)| said(Some(value.as_str())))
}

/// Which hub this invocation is addressing, asked once and in one place.
///
/// Three answers, in the order of how specific the claim is:
///
/// 1. what the caller passed — somebody said it outright;
/// 2. `ADJUTANT_HUB` — this process was started by a hub, so it *is* that hub; a hub
///    running a command inside a worker's worktree is still itself — except for the worker
///    itself, which follows its record when that names another hub (see `hub_id_with`);
/// 3. the worker record in the worktree we are standing in — nobody said anything and
///    nothing launched us, so the answer is whoever dispatched this worktree.
///
/// The third is what keeps `adj-report`'s promise that a worker never writes down an
/// address. The worker's agent is told to send, not to say where; `adj work` wrote the
/// answer into the worktree when it opened the tab, and it is read back from there. It
/// answers the question "where do I send", which is why only the commands that send, list
/// or name an inbox ask it — `hub_id_told` is the one for the other side.
///
/// There is a fourth outcome, and it is an error rather than a fourth answer: a record that
/// is there and cannot be read. See `worker_hub`. Being told outright is settled before the
/// record is opened at all, so a damaged record never takes the way out with it — `--hub`,
/// or `ADJUTANT_HUB`, still addresses whatever the caller names.
pub fn hub_id(explicit: Option<&str>, start: Option<&Path>) -> Result<Option<String>, String> {
    hub_id_with(explicit, start, is_self_or_descendant_of)
}

/// `hub_id`, with the question "is this process at or below that pid" handed in, so the order
/// of the answers can be pinned without starting a process tree.
///
/// One case sits between the flag and `ADJUTANT_HUB`: this process *is* the worker the
/// worktree's record names (or its MCP server, or a shell under it). Such a process was
/// started with whatever `ADJUTANT_HUB` said at the time, and a board that has since linked
/// the worker to another hub rewrote the record and not the environment of a running agent —
/// so the record is the newer word, and its saying "no hub" means the repository's own. It is
/// only asked when the environment names a hub the record does not, which is not the common
/// case, and a hub running a command in the worktree is not below the worker, so it stays
/// itself. A record that cannot be read never gets in the way here: the environment settled
/// this before it was opened, and still does.
pub(super) fn hub_id_with(
    explicit: Option<&str>,
    start: Option<&Path>,
    is_ancestor: impl Fn(u32) -> bool,
) -> Result<Option<String>, String> {
    if let told @ Some(_) = said(explicit) {
        return Ok(told);
    }
    let Some(inherited) = said(std::env::var(HUB_ENV).ok().as_deref()) else {
        return Ok(worker_hub(start)?.and_then(|record| record.hub));
    };
    if let Ok(Some(record)) = worker_hub(start)
        && record.hub.as_deref() != Some(inherited.as_str())
        && record.pid.is_some_and(is_ancestor)
    {
        return Ok(record.hub);
    }
    Ok(Some(inherited))
}

/// The hub named by the worker record of whichever worktree `start` is inside.
///
/// `current_worktree` rather than a walk of our own: it asks git, so a command run three
/// directories down inside a worktree gets the same answer as one run at its root, and a
/// main checkout answers with itself — where there is no worker record, which is the
/// correct "nobody dispatched me".
///
/// No record and an unreadable one are kept apart, because they are opposite answers. No
/// record means nobody dispatched this worktree and the repository's own hub is right. A
/// record that cannot be read means somebody did dispatch it and the address has been lost:
/// answering the repository's own hub there is the silent misroute this whole arrangement
/// exists to prevent, with every report the worker files landing in an inbox that may have
/// no hub reading it. So it is raised, and the caller can still say where to send.
///
/// `read_worker` draws the same line for the same reason and is deliberately not reused:
/// it also insists on a usable pid, which is its caller's question — whether a worktree may
/// be taken apart — and has nothing to do with where a report goes.
fn worker_hub(start: Option<&Path>) -> Result<Option<RecordedHub>, String> {
    let Some(worktree) = current_worktree(start) else {
        return Ok(None);
    };
    let path = worker_record_path(Path::new(&worktree));
    // `record_exists` rather than `exists`, which answers "no" to every error it meets — and
    // "no" here is the answer that loses the address.
    match record_exists(&path) {
        Ok(false) => return Ok(None),
        Err(e) => return Err(unreadable_record(&path, &e.to_string())),
        Ok(true) => {}
    }
    let record = match read_worker_record(Path::new(&worktree)) {
        Recorded::Absent => return Ok(None),
        Recorded::Unreadable => {
            return Err(unreadable_record(
                &path,
                "it is not the JSON this tool writes",
            ));
        }
        Recorded::Found(record) => record,
    };
    if record.hub_is_not_a_name() {
        return Err(unreadable_record(&path, "its hub is not a name"));
    }
    // Absent is a worker the repository's own hub dispatched, which records no identifier at
    // all. Null is read the same way rather than refused: the key is absent in what this
    // version writes, and a record has to read the same to every other version of this tool
    // on the machine.
    let hub = said(record.hub.as_deref());
    let pid = record.usable_pid();
    Ok(Some(RecordedHub { hub, pid }))
}

/// A worktree was dispatched and the record no longer says by whom. Named, because the one
/// thing that gets someone out of it is saying the identifier themselves.
fn unreadable_record(path: &Path, why: &str) -> String {
    format!(
        "cannot read the worker record at {}: {why}. It says which hub this worktree \
         reports to; pass the identifier (--hub, or ADJUTANT_HUB) to address one anyway",
        path.display()
    )
}
