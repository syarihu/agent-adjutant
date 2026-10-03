use super::*;

// ── presence ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubStatus {
    pub slug: String,
    pub hub_name: String,
    pub present: bool,
    pub pid: Option<u32>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    /// A record was found but the process behind it is gone.
    pub stale: bool,
}

/// The answer to "may this process be the hub for `slug`?".
#[derive(Debug)]
pub enum Claim {
    /// The record is ours. The caller may go on to become the agent. Where it landed is
    /// `hub_record_path`, so the claim does not need to hand it back.
    Ours,
    /// Another hub holds the record and is alive. Its status, so the caller can say which.
    Taken(Box<HubStatus>),
}

/// Record this process as the hub for `slug`, unless another live hub already is.
///
/// Called by `adj hub`, which then `exec`s the agent — so the PID stays valid across
/// the handover and the record points at the live agent process rather than at a launcher
/// that has already exited.
///
/// "One hub per repository" is the whole point of the record, and asking `hub_status`
/// first and writing second cannot enforce it: both askers hear "nobody home" and both
/// write. The record is therefore *claimed* — created with `create_new`, which the
/// filesystem refuses for the loser — and only a claim that was refused goes on to ask
/// whose it is.
///
/// What the loser asks is deliberately not `hub_status`. A hub that has just won the claim
/// has not `exec`ed the agent yet, so for a moment its command line is still the launcher's
/// and the name is not in it — `hub_status` calls that stale, and clearing a "stale" record
/// and retrying is the same check-then-act this is here to remove, arrived at from the
/// other side. The only question that can be asked safely is the one that cannot be
/// mid-change: is the recorded process still the process that was recorded? Alive with the
/// same start time means someone holds the name, whether or not they look like a hub yet.
/// A few retries, not a thousand: each one is two calls to `ps`, and a record that keeps
/// coming back means someone else keeps winning it.
pub fn claim_hub(
    slug: &str,
    hub_name: &str,
    cwd: &str,
    name_in_command: bool,
    hub: Option<&str>,
    terminal: Option<&crate::session::SessionTerminal>,
) -> Result<Claim, String> {
    let path = hub_record_path(slug);
    let mut record = json!({
        "pid": std::process::id(),
        "hubName": hub_name,
        "cwd": cwd,
        "startedAt": utc_stamp(now_secs()),
        "psStarted": ps_started(std::process::id()),
        "nameInCommand": name_in_command,
    });
    if let Some(hub) = said(hub)
        && let Some(fields) = record.as_object_mut()
    {
        fields.insert("hub".to_string(), json!(hub));
    }
    // Where this hub runs, so that something outside its tab — the board's stop button — can
    // find its pane without guessing from the pid. Absent for a record written before this,
    // which readers fall back from.
    if let Some(terminal) = terminal
        && let Some(fields) = record.as_object_mut()
        && let Ok(value) = serde_json::to_value(terminal)
    {
        fields.insert("terminal".to_string(), value);
    }
    match create_new_json(&path, &record) {
        Ok(()) => return Ok(Claim::Ours),
        Err(CreateError::Taken) => {}
        Err(CreateError::Failed(message)) => return Err(message),
    }
    match holder(&path) {
        Liveness::Alive => return Ok(Claim::Taken(Box::new(hub_status(slug, hub_name)))),
        Liveness::CannotTell => return Err(cannot_tell(&path)),
        Liveness::Gone => {}
    }

    // The record belongs to a process that has gone, and taking it over means deleting a
    // file and creating it again — two steps, which two launchers can interleave: the
    // second delete removes the *first one's live record* and the name is handed out
    // twice. Neither `create_new` nor `rename` prevents that, because the second launcher
    // is acting on a name whose contents changed underneath it, and POSIX has no "remove
    // this file only if it is still the one I looked at".
    //
    // So the takeover — and only the takeover — is serialised. The lock is a file nobody
    // can create twice, it names who holds it, and it is held for the few syscalls between
    // "this record is dead" and "this record is mine". A lock left behind by a crash goes
    // stale on a clock, which is safe here in a way it would never be for the hub record
    // itself: this one is held for microseconds, so an old one is evidence, not a guess.
    take_over(&path, &record, slug, hub_name)
}

fn take_over(path: &Path, record: &Value, slug: &str, hub_name: &str) -> Result<Claim, String> {
    // An advisory lock held on an open file, not a file whose existence is the lock.
    //
    // A lock made of a file has to answer "what if its holder died holding it", and every
    // answer to that is a guess — a clock, a pid, another liveness check — and every guess
    // is check-then-act again, one level down: two launchers both decide a lock is stale,
    // both break it, and both are inside. This one is released by the operating system when
    // the process ends, however it ends, so there is no stale case to reason about. The
    // file itself is never removed: unlinking it while another process holds it open would
    // hand the next two callers two different locks.
    let lock_path = path.with_extension("claiming");
    // Someone else is part-way through taking this name. Whatever they end up with, it is
    // not ours — the same answer we would have been given by arriving after they finished.
    let Some(lock) = crate::infra::fs::try_lock(&lock_path)? else {
        return Ok(Claim::Taken(Box::new(hub_status(slug, hub_name))));
    };

    // Asked again inside the lock: the record may have been taken over while we were
    // getting in, and the answer from outside is the one that was about to go stale.
    let claimed = match holder(path) {
        Liveness::Alive => Ok(Claim::Taken(Box::new(hub_status(slug, hub_name)))),
        Liveness::CannotTell => Err(cannot_tell(path)),
        Liveness::Gone => {
            remove_if_present(path)?;
            match create_new_json(path, record) {
                Ok(()) => Ok(Claim::Ours),
                Err(CreateError::Taken) => Ok(Claim::Taken(Box::new(hub_status(slug, hub_name)))),
                Err(CreateError::Failed(message)) => Err(message),
            }
        }
    };
    drop(lock);
    claimed
}

fn cannot_tell(path: &Path) -> String {
    format!(
        "cannot tell whether the hub recorded in {} is still running, so its name is left alone",
        path.display()
    )
}

/// The three answers `claim_hub` acts on, for a caller that is going to act on "nobody is
/// there" without claiming anything.
///
/// `hub_status` folds "cannot tell" into `present: false`, which is the right reading for a
/// sender — a message left for a hub that may or may not be there waits in a file until
/// somebody reads it, so guessing wrong costs nothing. It is the wrong reading for a caller
/// about to *start* a hub, and `claim_hub` is where that is normally caught. A route that
/// starts a hub somewhere else, and so never reaches a claim, has to ask here instead.
///
/// Deliberately `holder`'s reading of a record that is *there* and not a stricter one. The
/// question being asked is whether the name is free to take, which is the question
/// `claim_hub` will ask about the same record a moment later; a caller that refused where
/// the claim proceeds would be two routes disagreeing about one record, which is the
/// disagreement this is here to close.
///
/// What has to be added to `holder` is the case it never sees. It is called by `claim_hub`
/// only once `create_new` has failed, so the file is known to exist and `read_json`
/// answering `None` can only mean unreadable. Asked cold, that same `None` is mostly the
/// ordinary "nothing has ever registered here" — so existence is established first, the way
/// `read_worker` establishes it, and only then is the record read.
pub fn hub_liveness(slug: &str) -> Liveness {
    let path = hub_record_path(slug);
    // `record_exists` rather than `exists`, which answers "no" to every error it meets — and
    // "no" is the answer that goes on to start a hub. A symlink pointing nowhere is
    // something here, the same thing the claim's own `hard_link` meets and refuses on.
    match record_exists(&path) {
        // No record is the same answer as a record whose process has gone: nobody holds
        // the name. It is what `create_new` is about to say by succeeding.
        Ok(false) => Liveness::Gone,
        Err(_) => Liveness::CannotTell,
        Ok(true) => holder(&path),
    }
}

/// Why a hub's name was left alone, in the words `claim_hub` refuses with.
///
/// A caller that asks `hub_liveness` first is refusing on the claim's behalf, so it says
/// what the claim would have said — same record named, same thing to go and look at.
pub fn hub_cannot_tell(slug: &str) -> String {
    cannot_tell(&hub_record_path(slug))
}

/// The start time a record offers as an anchor, or `None` when what it offers cannot be one.
///
/// Four readers compare this against what `ps` says now, and a value that can match nothing
/// is worse than a missing one: blank, the comparison fails against every live process, and
/// each of them concludes "not the process I recorded". That is `Gone` at a hub's claim,
/// absent at either presence check, and a live worker's worktree offered up for deletion.
///
/// `ps_started` never writes a blank one, so this is about records written or edited by
/// something else. Interpreted here, once, so that an anchor which cannot anchor is the
/// same answer everywhere as no anchor at all — and each reader then falls back to whatever
/// it uses when a record has none, rather than asserting an absence it never established.
pub(super) fn recorded_anchor(record: &Value) -> Option<&str> {
    record
        .get("psStarted")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|started| !started.is_empty())
}

/// Whether the process a record names is still there — with "cannot tell" kept separate.
///
/// Start time and pid only, never the name in the command line: a record is written by a
/// launcher that has not yet `exec`ed the agent, so for that moment the name is genuinely
/// absent from a perfectly live hub, and mistaking that for a dead one is how a second hub
/// gets started.
pub(super) fn holder(path: &Path) -> Liveness {
    let Some(record) = read_json(path) else {
        // Unreadable rather than absent — `read_json` cannot say which, and a record that
        // cannot be read cannot be shown to belong to anybody. Refusing to act on it is
        // the answer that never takes a live hub's name.
        return Liveness::CannotTell;
    };
    let Some(pid) = record.get("pid").and_then(Value::as_u64) else {
        // A record with no pid names nobody. Nothing to be careful of.
        return Liveness::Gone;
    };
    let pid = pid as u32;
    let recorded = recorded_anchor(&record);
    match ps_answer(pid, "lstart") {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(started) => match recorded {
            // Same pid, different start time: the pid was recycled onto something else.
            Some(recorded) if started != recorded => Liveness::Gone,
            // Nothing to compare, so nothing here says this pid was recycled — and this
            // answer is the one that decides whether a name is free to take. The trade is
            // deliberate: an anchorless record over a pid that is alive but unrelated makes
            // the name look taken for as long as that process lives, and `adj hub-stop`
            // clears it. Read the other way, a running hub loses its name to the next
            // launcher and both then answer to the same address, which is the one failure
            // this protocol exists to prevent. It is the same trade `close` makes on the
            // worker side: what cannot be established is left alone, and a person finishes
            // the job.
            _ => Liveness::Alive,
        },
    }
}

/// Public because `close` needs the same three answers about a worker that `claim_hub`
/// needs about a hub, and for the same reason: it acts destructively on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    Gone,
    /// `ps` could not be run, or the record could not be read. Not evidence of anything.
    CannotTell,
}

pub fn unregister_hub(slug: &str) -> Result<(), String> {
    remove_if_present(&hub_record_path(slug))
}

/// Remove the hub record only while it still names the process `pid` started at `started`.
/// `Ok(true)` when the record is gone afterwards — removed, or already absent — and
/// `Ok(false)` when it names another process, which registered since and is not this call's
/// to clear.
///
/// Under the lock a takeover takes, so that it cannot read a record a claim is part-way
/// through replacing: unlike the launcher's own cleanup, whoever calls this is acting on a
/// hub it did not start, some time after it looked.
pub fn unregister_hub_if(slug: &str, pid: u32, started: Option<&str>) -> Result<bool, String> {
    let path = hub_record_path(slug);
    let lock_path = path.with_extension("claiming");
    let _lock = crate::infra::fs::lock(&lock_path)?;
    let named = match read_json(&path) {
        None if !path.exists() => return Ok(true),
        None => return Ok(false),
        Some(record) => record,
    };
    let same = named.get("pid").and_then(Value::as_u64) == Some(u64::from(pid))
        && recorded_anchor(&named) == started.map(str::trim).filter(|s| !s.is_empty());
    if !same {
        return Ok(false);
    }
    remove_if_present(&path)?;
    Ok(true)
}

/// Remove the hub record only while it still names no process. `Ok(true)` when it is gone
/// afterwards — removed, or already absent — and `Ok(false)` when it names a pid (a hub
/// claimed the name since the caller looked) or cannot be read.
///
/// Under the same lock as `unregister_hub_if`, for a caller that has stopped or checked a
/// hub and must not delete the record of one that registered in the meantime.
pub fn unregister_hub_if_unnamed(slug: &str) -> Result<bool, String> {
    let path = hub_record_path(slug);
    let lock_path = path.with_extension("claiming");
    let _lock = crate::infra::fs::lock(&lock_path)?;
    match read_json(&path) {
        None if !path.exists() => Ok(true),
        None => Ok(false),
        Some(record) if record.get("pid").is_some_and(|pid| !pid.is_null()) => Ok(false),
        Some(_) => remove_if_present(&path).map(|()| true),
    }
}

/// Whether the process a hub record named is still that process, without going through the
/// record: for a caller that has read the record once and must go on asking about the same
/// hub after another has claimed the name. A record with no anchor names a live pid as
/// alive, as `holder` reads it.
pub fn hub_process_liveness(pid: u32, started: Option<&str>) -> Liveness {
    match ps_answer(pid, "lstart") {
        Answer::NoSuchProcess => Liveness::Gone,
        Answer::CannotTell => Liveness::CannotTell,
        Answer::Said(now) => match started.map(str::trim).filter(|s| !s.is_empty()) {
            Some(recorded) if recorded != now => Liveness::Gone,
            _ => Liveness::Alive,
        },
    }
}

pub fn hub_status(slug: &str, hub_name: &str) -> HubStatus {
    hub_status_with(&ProcessTable::each(), slug, hub_name)
}

/// `hub_status`, asking `table` when the hub's process started.
pub fn hub_status_with(table: &ProcessTable, slug: &str, hub_name: &str) -> HubStatus {
    let mut status = HubStatus {
        slug: slug.to_string(),
        hub_name: hub_name.to_string(),
        present: false,
        pid: None,
        cwd: None,
        started_at: None,
        stale: false,
    };
    let Some(record) = read_json(&hub_record_path(slug)) else {
        return status;
    };
    status.pid = record.get("pid").and_then(Value::as_u64).map(|p| p as u32);
    status.cwd = record
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    status.started_at = record
        .get("startedAt")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ps_started = recorded_anchor(&record);
    // Looking for the hub's name in its command line is the stronger of the two anchors,
    // but it only works if the name is *there* — a `hubRunner` with no `{name}` in it, or
    // one that `exec`s something that keeps none of its arguments, produces a live hub that
    // fails this check forever. So the launcher records whether the name it was about to
    // run actually carried it, and a session that could never match is matched on its
    // start time alone, exactly as a worker is. A record from before this was written has
    // no answer, and the old behaviour is the safe reading of that: it was started by a
    // template that did carry the name, or it would not have been found at all.
    let named = record
        .get("nameInCommand")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let recorded_name = record
        .get("hubName")
        .and_then(Value::as_str)
        .unwrap_or(hub_name);
    let expect = named.then_some(recorded_name);
    match status.pid {
        Some(pid) if process_matches_with(table, pid, expect, ps_started) => status.present = true,
        _ => status.stale = true,
    }
    status
}

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

/// Blank is silence. An identifier that is empty or only spaces is a caller passing the
/// flag through without a value, and reading it as a hub called "" builds an address nobody
/// can type a second time.
pub(super) fn said(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|hub| !hub.is_empty())
        .map(str::to_string)
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
    let Some(record) = read_json(&path) else {
        return Err(unreadable_record(
            &path,
            "it is not the JSON this tool writes",
        ));
    };
    let Some(record) = record.as_object() else {
        return Err(unreadable_record(&path, "it is not an object"));
    };
    let hub = match record.get("hub") {
        // Absent is a worker the repository's own hub dispatched, which records no
        // identifier at all. Null is read the same way rather than refused: the key is
        // absent in what this version writes, and a record has to read the same to every
        // other version of this tool on the machine.
        None | Some(Value::Null) => None,
        Some(Value::String(hub)) => said(Some(hub)),
        Some(_) => return Err(unreadable_record(&path, "its hub is not a name")),
    };
    let pid = record
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 0);
    Ok(Some(RecordedHub { hub, pid }))
}

/// What a worker record says about where its worker reports: `None` for a repository's own
/// hub. Kept apart from "no record", which `worker_hub` answers with `None` around this.
struct RecordedHub {
    hub: Option<String>,
    /// The process that then `exec`s the agent, so the agent and the servers it starts are it
    /// or below it.
    pid: Option<u32>,
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

pub fn status_json(status: &HubStatus) -> Value {
    json!({
        "hubName": status.hub_name,
        "slug": status.slug,
        "present": status.present,
        "stale": status.stale,
        "pid": status.pid,
        "cwd": status.cwd,
        "startedAt": status.started_at,
        "inbox": inbox_dir(&status.slug).to_string_lossy(),
    })
}
