//! Passing chosen review comments on to Jules, in the person's own name.

use super::*;

use serde_json::{Value, json};

use crate::task;

/// How long `relay` waits for `gh` to post: it holds the task's lock, and a board connection
/// thread waits on it.
const POST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub fn relay(
    ctx: &crate::registry::Context,
    id: &str,
    chosen: &[Chosen],
    note: Option<&str>,
) -> Result<Value, String> {
    if chosen.is_empty() {
        return Err("choose at least one comment to pass on".to_string());
    }
    // Once each: a comment named twice would be posted twice in one relay.
    for (n, c) in chosen.iter().enumerate() {
        if chosen[..n].iter().any(|earlier| earlier.id == c.id) {
            return Err(format!("comment {} is named more than once", c.id));
        }
    }
    // Held from the check to the save, across both round trips to GitHub. Two relays of one
    // comment — a double click, the board and a shell at once — would otherwise both find it
    // not yet passed on and both post it. Waiting a few seconds on a button somebody pressed
    // is the cheaper failure.
    let lock = task::lock(ctx, id)?;
    let all = findings(ctx, id)?;
    let mut picked = Vec::new();
    for want in chosen {
        let found = all.iter().find(|f| f.id == want.id).ok_or(format!(
            "no review comment {} on this pull request",
            want.id
        ))?;
        if found.relayed {
            return Err(format!("comment {} has already been passed on", want.id));
        }
        picked.push((found, want.note.as_deref()));
    }
    let task = task::get(&ctx.state, &ctx.repo.slug, id)?;
    let pr = task
        .pr
        .clone()
        .ok_or(format!("{id} has no pull request yet"))?;
    // Jules acts on the comments of the account that started it and nobody else's. Posted
    // from another, the comment would go up, be marked as passed on, and be ignored.
    //
    // Asked afresh rather than from the board's cache: the refusal below tells the person to
    // `gh auth switch`, and a board that remembered the old account would go on refusing — or,
    // switched the other way, let the comment go up in the wrong name.
    if let Some(by) = &task.jules_by {
        let me = github_login(&ctx.repo.main)
            .ok_or("cannot tell which GitHub account gh is signed in as (`gh auth status`)")?;
        if &me != by {
            return Err(format!(
                "gh is signed in as {me}, but {by} started this Jules session and Jules answers only {by}: switch accounts with `gh auth switch`"
            ));
        }
    }
    let body = relay_body(&picked, note);
    // On stdin: the text is the reviewers' and the person's, and neither belongs on a
    // command line. Given up on after `POST_TIMEOUT`: the task lock is held, and a `gh` that
    // hangs would hold it, and the board's request, for good.
    let run = crate::infra::gh::run_with_input(
        Some(&ctx.repo.main),
        &["pr", "comment", &pr, "--body-file", "-"],
        Some(body.as_bytes()),
        std::time::Instant::now() + POST_TIMEOUT,
    )
    .map_err(|e| {
        format!(
            "gh could not post the comment: {e}; it may still have gone up, so check {pr} before passing these on again"
        )
    })?;
    if !run.ok {
        return Err(format!("gh could not post the comment: {}", run.stderr));
    }
    let posted = run.stdout.trim().to_string();
    // Written after the comment is up, so a failure to post leaves them choosable.
    let mut task = task::get(&ctx.state, &ctx.repo.slug, id)?;
    for (f, _) in &picked {
        if !task.relayed.contains(&f.id) {
            task.relayed.push(f.id.clone());
        }
    }
    task.updated_at = crate::infra::clock::utc_stamp(crate::infra::clock::now_secs());
    task::save(ctx, &task)?;
    drop(lock);
    let ids: Vec<&str> = chosen.iter().map(|c| c.id.as_str()).collect();
    Ok(json!({ "relayed": ids, "comment": posted }))
}
