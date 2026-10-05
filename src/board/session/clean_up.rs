use std::path::Path;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use crate::board::view::{find_session, git_state_of};
use crate::board::{NEXT, Server, input_of, settings_now, text};
use crate::infra::paths::same_path;
use crate::infra::template::{Sub, render};
use crate::kernel::worktree_state::GitState;
use crate::mail;
use crate::registry::{self, Context};
use crate::task::{self, Executor, Status};

// ── cleanup ──────────────────────────────────────────────────────────

/// Remove the worktree of the worker session `id` and its local branch, closing the session
/// first if it runs.
///
/// The hub's procedure says it is the only route by which anything is removed; from here the
/// board is a second one, and makes the same checks itself rather than asking the hub, which
/// may not be running. Refusals that cannot be forced (the ground under a worker still to
/// come) are errors. What would be lost is a `removed: false` with the reasons, answered 200
/// like `closed: false` is: the page keeps only `error` from a non-2xx answer, and the reasons
/// are what the person needs to decide whether to force.
pub fn cleanup(server: &Server, id: &str, body: &[u8]) -> Result<Value, String> {
    let input = input_of(body)?;
    let force = match input.get("force") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(force)) => *force,
        Some(other) => return Err(format!("force has to be true or false, not {other}")),
    };
    let confirm = text(&input, "confirm")?;
    let settings = settings_now(server);
    let session = find_session(server, &settings, id)?;
    let repo = &server.ctx.repo;
    if session.kind != "worker" {
        return Err("only a worker session has a worktree to remove".to_string());
    }
    let worktree = session.worktree.as_str();
    if same_path(worktree, &repo.main) {
        return Err("the main checkout is not a worktree to remove".to_string());
    }
    let listed = crate::kernel::identity::linked_worktrees(&repo.main)?;
    if !listed.iter().any(|w| same_path(w, worktree)) {
        return Err(format!("not a worktree of this repository: {worktree}"));
    }
    let name = Path::new(worktree)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if force && confirm != Some(name.as_str()) {
        return Err(format!("confirm has to be the worktree's name: {name}"));
    }

    // The tasks that name this worktree, in whichever hub they were made.
    let hubs = mail::all_repo_hubs(&server.ctx.state, repo);
    let mut tasks: Vec<(String, task::Task)> = Vec::new();
    for hub in &hubs {
        for t in task::list(&server.ctx.state, &hub.slug) {
            if t.worktree
                .as_deref()
                .is_some_and(|w| same_path(w, worktree))
            {
                tasks.push((hub.slug.clone(), t));
            }
        }
    }
    if tasks.iter().any(|(_, t)| t.status == Status::Queued) {
        return Err("a queued task is waiting in it".to_string());
    }
    // A Jules plan is written in a detached worktree with no commits, which looks finished.
    if tasks.iter().any(|(_, t)| {
        t.executor == Executor::Jules && t.status == Status::Dispatched && t.jules_session.is_none()
    }) {
        return Err("a Jules plan is being written in it".to_string());
    }
    if registry::is_starting(Path::new(worktree), crate::infra::clock::now_secs()) {
        return Err("the session is starting".to_string());
    }

    let state = git_state_of(server, &session);
    let reasons = loss_reasons(&state);
    if !reasons.is_empty() && !force {
        return Ok(json!({
            "removed": false,
            "reasons": reasons,
            "git": state.as_ref().ok().and_then(|s| s.as_ref()),
        }));
    }
    let state = state.ok().flatten();

    let main = Path::new(&repo.main);
    let was_running = registry::worker_status(Path::new(worktree)).present;
    // `close` finds the worktree free when no worker is there, so a stopped session is not
    // refused; `false` is a worker that may still be running, or a record that cannot be
    // read, and removing under either is what this must not do. Outside the dispatch lock,
    // which others give up waiting for after ten seconds.
    //
    // Only when something may be running: closing a worker that is gone clears its record, and
    // a removal that then fails would have taken the session's phase history for nothing.
    let may_run = match registry::read_worker(Path::new(worktree)) {
        registry::Recorded::Absent => false,
        registry::Recorded::Unreadable => true,
        registry::Recorded::Found(worker) => {
            registry::worker_liveness(&worker) != registry::Liveness::Gone
        }
    };
    if may_run && !crate::lifecycle::worker::close(&settings, Path::new(worktree), false)?.is_free()
    {
        return Err("the session could not be closed; nothing was removed".to_string());
    }
    // Looked at again now that nothing is running: a worker that committed between the first
    // look and its tab closing has made work the first look did not see, and `branch -D`
    // below would drop it.
    if !force {
        let again = git_state_of(server, &session);
        let reasons = loss_reasons(&again);
        if !reasons.is_empty() {
            return Ok(json!({
                "removed": false,
                "closed": was_running,
                "reasons": reasons,
                "git": again.ok().flatten(),
            }));
        }
    }
    // The lock covers only the last look at the worker slots and the marker that says the
    // worktree is going, which `claim_worker_slot` refuses on under the same lock: a worker
    // that starts here from now on is turned away. The removal itself, which can take long on
    // a big worktree, runs after the lock is released, so that a dispatch elsewhere is not
    // made to wait for it.
    let removing = registry::with_dispatch_lock(main, || -> Result<_, String> {
        if registry::holds_worker_slot(Path::new(worktree), crate::infra::clock::now_secs()) {
            return Err("a session started in it meanwhile; nothing was removed".to_string());
        }
        registry::mark_worktree_removing(main, Path::new(worktree))
    })??;
    let removed = remove_worktree(&server.ctx.state, &repo.main, worktree, force);
    // Released only now, whatever the removal came to.
    drop(removing);
    removed?;

    // The worker's record, its saved session and its outbox live inside the worktree, so the
    // session is off the board with it; nothing of it is kept in the state directory.
    let branch_name = state
        .as_ref()
        .and_then(|s| s.branch.clone())
        .or(session.branch.clone());
    let branch = match &branch_name {
        Some(branch) => match delete_branch(&repo.main, branch) {
            Ok(()) => json!({ "name": branch, "deleted": true }),
            Err(e) => json!({ "name": branch, "deleted": false, "error": e }),
        },
        None => json!({ "name": Value::Null, "deleted": false }),
    };

    let hooks = run_remove_hooks(server, worktree, &name);

    // After the removal, as the hub does: a card left in progress for a worktree that is gone
    // would stay on the board for good.
    let mut done = Vec::new();
    let mut task_errors = Vec::new();
    for (slug, t) in tasks
        .iter()
        .filter(|(_, t)| matches!(t.status, Status::Dispatched | Status::Pr))
    {
        let Some(hub) = hubs.iter().find(|h| &h.slug == slug) else {
            continue;
        };
        let mut addressed = repo.clone();
        addressed.slug = hub.slug.clone();
        addressed.hub_name = hub.name.clone();
        let ctx = Context {
            repo: addressed,
            resolved: server.ctx.resolved.clone(),
            state: server.ctx.state.clone(),
            settings: settings.clone(),
        };
        let done_patch = task::TaskPatch {
            status: Some(Status::Done),
            ..task::TaskPatch::default()
        };
        match task::update(&ctx, &t.id, &done_patch, false) {
            Ok(_) => done.push(t.id.clone()),
            Err(e) => task_errors.push(json!({ "id": t.id, "error": e })),
        }
    }
    let mut reply = json!({
        "removed": true,
        "forced": force,
        "closed": was_running,
        "branch": branch,
        "tasks": done,
        "hooks": hooks,
    });
    if !task_errors.is_empty() {
        reply["taskErrors"] = json!(task_errors);
    }
    Ok(reply)
}

/// What removing the worktree would lose, as `{kind, detail}` for the page to list. A git that
/// could not answer is a reason of its own: not knowing is not the same as nothing to lose.
fn loss_reasons(state: &Result<Option<GitState>, String>) -> Vec<Value> {
    let state = match state {
        Err(e) => return vec![json!({ "kind": "git", "detail": e })],
        // The directory is gone: there is nothing left to lose.
        Ok(None) => return Vec::new(),
        Ok(Some(state)) => state,
    };
    let mut reasons = Vec::new();
    let uncommitted = &state.uncommitted;
    if uncommitted.files > 0 {
        reasons.push(json!({
            "kind": "uncommitted",
            "detail": format!(
                "{} changed file(s), +{} -{} lines",
                uncommitted.files, uncommitted.insertions, uncommitted.deletions
            ),
        }));
    }
    if uncommitted.untracked > 0 {
        reasons.push(json!({
            "kind": "untracked",
            "detail": format!("{} untracked path(s)", uncommitted.untracked),
        }));
    }
    if state.unpushed.count > 0 {
        reasons.push(json!({
            "kind": "unpushed",
            "detail": format!("{} commit(s) no remote has", state.unpushed.count),
        }));
    }
    reasons
}

/// `git worktree remove`, forced only when the person forced it.
///
/// Adjutant's own files in the worktree (the worker's record, its saved session, the brief) are
/// untracked in a repository that does not ignore `.claude`, and git refuses to remove a
/// worktree with untracked files. Only when git says no are the ones no commit tracks moved out
/// of the way for a second try, and they are put back if that fails too: until the worktree is
/// gone they are what lets the session be seen and resumed. A directory that is already gone
/// is removed by name, which drops git's record of that one worktree and no other.
fn remove_worktree(root: &Path, main: &str, worktree: &str, force: bool) -> Result<(), String> {
    let mut args = vec!["-C", main, "worktree", "remove"];
    if force || !Path::new(worktree).is_dir() {
        args.push("--force");
    }
    args.push(worktree);
    let first = match git_ok(&args) {
        Ok(()) => return Ok(()),
        Err(e) if force => return Err(e),
        Err(e) => e,
    };
    // git's own message is the one that says why, so it is what is returned.
    let aside = set_aside_own_files(root, worktree)
        .map_err(|e| format!("{first} (and adjutant's files could not be moved aside: {e})"))?;
    match git_ok(&args) {
        Ok(()) => {
            aside.discard();
            Ok(())
        }
        Err(e) => match aside.restore() {
            Ok(()) => Err(e),
            Err(kept) => Err(format!("{e} ({kept})")),
        },
    }
}

/// Files moved out of a worktree: where each was, and the copy kept outside it.
struct Aside {
    files: Vec<(std::path::PathBuf, std::path::PathBuf)>,
}

impl Aside {
    /// Put back every file whose place is empty. The directory holding the copies goes only
    /// when all of them are back; otherwise it stays and the error says where.
    fn restore(self) -> Result<(), String> {
        let mut failed = false;
        for (original, kept) in &self.files {
            // A file that is there is the worker's own, possibly newer: never overwritten.
            if !original.exists() && move_file(kept, original).is_err() {
                failed = true;
            }
        }
        match (failed, self.dir()) {
            (false, dir) => {
                if let Some(dir) = dir {
                    let _ = std::fs::remove_dir_all(dir);
                }
                Ok(())
            }
            (true, dir) => Err(format!(
                "adjutant's files could not all be put back; they are kept in {}",
                dir.map(|d| d.display().to_string()).unwrap_or_default()
            )),
        }
    }

    fn discard(self) {
        if let Some(dir) = self.dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn dir(&self) -> Option<&Path> {
        self.files.first().and_then(|(_, kept)| kept.parent())
    }
}

/// Rename, or copy and remove where the two places are on different file systems. A copy that
/// fails part way is removed, so that it cannot later stand in for the original.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if let Err(e) = std::fs::copy(from, to) {
        let _ = std::fs::remove_file(to);
        return Err(e);
    }
    // A copy that stays beside a source that could not be removed would be a file the list of
    // what was moved does not know.
    std::fs::remove_file(from).inspect_err(|_| {
        let _ = std::fs::remove_file(to);
    })
}

/// Move `.claude/adjutant-*` and `.claude/task-brief.md` out of `worktree` unless git tracks
/// them, into a directory of the state directory made when the first one is moved.
fn set_aside_own_files(root: &Path, worktree: &str) -> Result<Aside, String> {
    let listed =
        crate::infra::git::git(&["-C", worktree, "ls-files", "-z", "--", ".claude"], None)?;
    if !listed.status.success() {
        return Err(format!(
            "cannot list the tracked files: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        ));
    }
    let tracked = String::from_utf8_lossy(&listed.stdout).to_string();
    let tracked: Vec<&str> = tracked.split('\0').collect();
    let mut aside = Aside { files: Vec::new() };
    let Ok(entries) = std::fs::read_dir(Path::new(worktree).join(".claude")) else {
        return Ok(aside);
    };
    // Read to the end before anything is moved, so that the listing is not of a directory
    // that is changing under it.
    let entries: Vec<_> = entries.flatten().collect();
    let dir = root.join(format!(
        "cleanup-{}-{}-{}",
        std::process::id(),
        crate::infra::clock::now_secs(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let mut made = false;
    for entry in entries {
        let file = entry.file_name().to_string_lossy().to_string();
        let ours = file.starts_with("adjutant-") || file == "task-brief.md";
        if !ours || tracked.contains(&format!(".claude/{file}").as_str()) {
            continue;
        }
        // `create_dir` for the first: a directory already there is not ours and is not written
        // into.
        let step = (if made {
            Ok(())
        } else {
            std::fs::create_dir_all(dir.parent().unwrap_or(&dir))
                .and_then(|_| std::fs::create_dir(&dir))
                .inspect(|_| made = true)
        })
        .and_then(|_| move_file(&entry.path(), &dir.join(&file)));
        match step {
            Ok(()) => aside.files.push((entry.path(), dir.join(&file))),
            Err(e) => {
                let why = format!("{}: {e}", entry.path().display());
                // The directory may have been made for a file that never got into it.
                if made && aside.files.is_empty() {
                    let _ = std::fs::remove_dir(&dir);
                }
                return match aside.restore() {
                    Ok(()) => Err(why),
                    Err(kept) => Err(format!("{why}; {kept}")),
                };
            }
        }
    }
    Ok(aside)
}

fn git_ok(args: &[&str]) -> Result<(), String> {
    let out = crate::infra::git::git(args, None)?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// The local branch only. A remote ref is never touched from here: the branch may be the
/// head of a PR that is still open.
fn delete_branch(main: &str, branch: &str) -> Result<(), String> {
    git_ok(&["-C", main, "branch", "-D", "--", branch])
}

/// The repository's `onWorktreeRemove` commands, run in the main checkout after the removal
/// with `{worktree}` and `{name}` filled in, as the hub does. Read from the config now, not
/// from the settings the server started with. A hook that fails is reported and does not
/// undo anything.
fn run_remove_hooks(server: &Server, worktree: &str, name: &str) -> Vec<Value> {
    let hooks = crate::kernel::config::resolve_config(&server.ctx.repo.nwo)
        .ok()
        .and_then(|resolved| resolved.config)
        .and_then(|config| config.get("onWorktreeRemove").cloned())
        .and_then(|hooks| hooks.as_array().cloned())
        .unwrap_or_default();
    hooks
        .iter()
        .filter_map(Value::as_str)
        .map(|template| {
            let command = render(
                template,
                &[
                    ("worktree", Sub::Quoted(worktree)),
                    ("name", Sub::Quoted(name)),
                ],
            );
            let ran = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(&server.ctx.repo.main)
                .output();
            match ran {
                Ok(out) if out.status.success() => json!({ "command": command, "ok": true }),
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    let error = match stderr.is_empty() {
                        true => format!("exited with {}", out.status),
                        false => stderr,
                    };
                    json!({ "command": command, "ok": false, "error": error })
                }
                Err(e) => json!({ "command": command, "ok": false, "error": e.to_string() }),
            }
        })
        .collect()
}
