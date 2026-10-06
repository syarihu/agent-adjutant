//! The review comments on a Jules task's pull request that could be passed on.

use super::*;

use crate::task;

/// The GitHub account `gh` is signed in as, or `None` when it cannot say.
///
/// Given up on after `LOGIN_TIMEOUT`: `gh` waiting on a login prompt or a network that has
/// gone should not hold up a command that has more to do.
pub fn github_login(main: &str) -> Option<String> {
    login(main).ok()
}

/// `github_login`, or an error that says why `gh` could not tell: it did not run or answer in
/// time, it exited with an error, or it printed nothing.
pub(super) fn login(main: &str) -> Result<String, String> {
    asked_login(main).map_err(|reason| {
        format!("cannot tell which GitHub account gh is signed in as (`gh auth status`): {reason}")
    })
}

fn asked_login(main: &str) -> Result<String, String> {
    let run = crate::infra::gh::run(
        Some(main),
        &["api", "user", "--jq", ".login"],
        std::time::Instant::now() + LOGIN_TIMEOUT,
    )?;
    if !run.ok {
        let stderr = run.stderr.trim();
        return Err(if stderr.is_empty() {
            "gh exited with an error".to_string()
        } else {
            stderr.to_string()
        });
    }
    let login = run.stdout.trim().to_string();
    if login.is_empty() {
        return Err("gh printed no login".to_string());
    }
    Ok(login)
}

/// How long `login` waits for `gh`. As long as the other `gh` deadlines: a loaded machine can
/// take seconds just to start it, while one hanging on a login prompt is still given up on.
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// How long `findings` waits for `gh`: a paginated listing can be several requests, and a
/// board connection thread waits on it.
const FINDINGS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The review comments on the task's pull request from the repository's review bots, oldest
/// first. Replies are left out: a thread is passed on by its first comment.
///
/// Given up on after `FINDINGS_TIMEOUT`, so a `gh` that hangs cannot hold a board request.
pub fn findings(ctx: &crate::registry::Context, id: &str) -> Result<Vec<Finding>, String> {
    let task = task::get(&ctx.state, &ctx.repo.slug, id)?;
    let pr = task
        .pr
        .as_deref()
        .ok_or(format!("{id} has no pull request yet"))?;
    // The number alone is asked about in this repository, so a URL pointing at another one
    // would list that number's comments here — and relay would post them to the other PR.
    let number = this_repos_pr(pr, &ctx.repo.nwo)
        .ok_or(format!("not a pull request of {}: {pr}", ctx.repo.nwo))?;
    let skip = not_findings_by(&ctx.repo.main)?;
    let endpoint = format!("repos/{}/pulls/{number}/comments", ctx.repo.nwo);
    let run = crate::infra::gh::run(
        Some(&ctx.repo.main),
        &[
            "api",
            &endpoint,
            "--paginate",
            "--jq",
            ".[] | {id, path, line, original_line, body, html_url, in_reply_to_id, user: .user.login}",
        ],
        std::time::Instant::now() + FINDINGS_TIMEOUT,
    )
    .map_err(|e| format!("gh could not list the review comments: {e}"))?;
    if !run.ok {
        return Err(format!(
            "gh could not list the review comments: {}",
            run.stderr
        ));
    }
    let listed = run.stdout;
    Ok(parse_findings(&listed, &skip, &task.relayed))
}

/// The number of `pr`, when it is a pull request of `nwo` on github.com.
///
/// A URL of another repository is refused: the number alone is asked about here, so it would
/// list that number's comments in this one. Names compare without case, as GitHub does.
pub(super) fn this_repos_pr(pr: &str, nwo: &str) -> Option<u64> {
    let r = task::pr_ref(pr, Some(("github.com", nwo)))?;
    (r.host == "github.com" && r.nwo().eq_ignore_ascii_case(nwo)).then_some(r.number)
}

/// Whose comments are not findings to pass on: Jules' own, and those of the account `gh` is
/// signed in as — Jules already reads that person's comments, which is why a relay is posted
/// in their name at all.
///
/// Everyone else is listed. Jules acts on the person who started it and on nobody else, so a
/// review bot, Copilot and a colleague all go unanswered alike. `reviewBots` is not the list:
/// it names the reviews a worker waits for, which is a different question, and a repository
/// that waits only for Copilot would otherwise never see CodeRabbit's findings here.
fn not_findings_by(main: &str) -> Result<Vec<String>, String> {
    Ok(vec![JULES_LOGIN.to_string(), signed_in(main)?])
}

/// Who `gh` is signed in as. An error rather than a guess when it cannot say: without it the
/// person's own comments would be listed, and passed on to a Jules that already read them.
///
/// Asked once per process and kept, since the board asks on every poll of a PR in review and
/// the answer does not change under it. A failure is not kept; the next call asks again.
fn signed_in(main: &str) -> Result<String, String> {
    static ME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let mut me = ME.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = me.as_ref() {
        return Ok(cached.clone());
    }
    let who = login(main)?;
    *me = Some(who.clone());
    Ok(who)
}
