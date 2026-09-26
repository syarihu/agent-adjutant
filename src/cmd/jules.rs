//! `adj jules` — hand a task's approved plan to Jules, and ask how that is going.
//!
//! The worker is the caller. It plans in its worktree as it always does, and once the plan
//! gate is answered it writes the design out for Jules and runs `adj jules start`, which
//! starts the session and writes its id onto the task record. From there the record is what
//! the board and the hub follow: the session id is the only thing about Jules kept locally.

use serde_json::{Value, json};

use super::task as tasks;
use crate::jules;
use crate::task;

pub struct StartArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: &'a str,
    /// Where the prompt is: a file, or `-` for stdin.
    pub prompt: &'a str,
    pub base: Option<&'a str>,
    pub json: bool,
}

pub fn start(args: &StartArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let task = task::load(&tasks::dir(&ctx), args.id)?;
    // A second session for one task is two pull requests for one change, and the first one
    // would be forgotten: the record keeps one id.
    if let Some(session) = &task.jules_session {
        return Err(format!(
            "{} is already with Jules (session {session}); clear it with `adj task update --id {} --jules-session ''` to start another",
            task.id, task.id
        ));
    }
    // Given explicitly, the branch is taken as it is. The task's own base is what was typed on
    // the board, where `origin/feature/x` is as likely as `feature/x`; Jules wants the name
    // GitHub has, so that one is checked against this checkout's remote-tracking refs.
    let base = match (args.base, task.base.as_deref()) {
        (Some(given), _) => given.to_string(),
        (None, Some(stored)) => branch_on_github(&ctx.repo.main, stored),
        (None, None) => {
            return Err("which branch should Jules start from? pass --base (the branch this worktree was cut from, without origin/)".to_string());
        }
    };
    let base = base.as_str();
    let prompt = read_prompt(args.prompt)?;
    let session = jules::create(
        &ctx.settings.jules_key,
        &ctx.repo.nwo,
        base,
        &task.title,
        &prompt,
    )?;
    // Written as soon as the session exists. If this fails the session is still running, so
    // the error says which one it is rather than leaving it to be found on jules.google.com.
    let (task, _) =
        tasks::update(&ctx, &task.id, &json!({ "julesSession": session.id })).map_err(|e| {
            format!(
                "Jules started session {} but the task record could not be updated: {e}",
                session.id
            )
        })?;
    if args.json {
        println!("{}", json!({ "task": task.id, "session": session }));
        return Ok(());
    }
    println!("{} — handed to Jules as session {}", task.id, session.id);
    if let Some(url) = &session.url {
        println!("{url}");
    }
    Ok(())
}

pub struct ShowArgs<'a> {
    pub repo: Option<&'a str>,
    pub hub: Option<&'a str>,
    pub id: Option<&'a str>,
    pub session: Option<&'a str>,
    pub json: bool,
}

/// How a session is doing, named by its task or by its own id.
pub fn show(args: &ShowArgs<'_>) -> Result<(), String> {
    let ctx = super::context(args.repo, args.hub)?;
    let session_id = match (args.session, args.id) {
        (Some(session), _) => session.to_string(),
        (None, Some(id)) => task::load(&tasks::dir(&ctx), id)?
            .jules_session
            .ok_or(format!("{id} has not been handed to Jules"))?,
        (None, None) => return Err("name the task with --id or the session with --session".into()),
    };
    let session = jules::get(&ctx.settings.jules_key, &session_id)?;
    if args.json {
        println!("{}", serde_json::to_value(&session).unwrap_or(Value::Null));
        return Ok(());
    }
    println!("{} {}", session.id, session.state);
    for line in [&session.url, &session.pr].into_iter().flatten() {
        println!("{line}");
    }
    Ok(())
}

/// The branch as GitHub names it, for a base written the way `git worktree add` takes it.
///
/// `origin/` is taken off only when this checkout says it is the remote's prefix: a
/// remote-tracking ref of that name exists, and no branch on the remote is itself called
/// `origin/…`. Anything the checkout cannot vouch for is left as it was written, for the API to
/// accept or refuse — stripping blindly would turn a real branch named `origin/x` into `x`.
fn branch_on_github(main: &str, base: &str) -> String {
    let Some(rest) = base.strip_prefix("origin/") else {
        return base.to_string();
    };
    let known = |name: &str| {
        crate::repo::git(
            &[
                "-C",
                main,
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{name}"),
            ],
            None,
        )
        .is_ok_and(|out| out.status.success())
    };
    if known(&format!("origin/{base}")) || !known(base) {
        base.to_string()
    } else {
        rest.to_string()
    }
}

/// The prompt, from a file or from stdin. The design is dozens of lines and does not belong
/// on a command line, where a quote in it would end the argument early.
fn read_prompt(from: &str) -> Result<String, String> {
    let text = match from {
        "-" => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            buf
        }
        path => {
            let path = crate::config::expand_home(path);
            std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        }
    };
    if text.trim().is_empty() {
        return Err("the prompt is empty".to_string());
    }
    Ok(text)
}
