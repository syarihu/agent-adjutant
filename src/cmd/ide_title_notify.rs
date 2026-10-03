use super::*;

pub fn open_ide(repo_arg: Option<&str>, worktree: &str, dry_run: bool) -> Result<(), String> {
    let ctx = context_without_hub(repo_arg)?;
    let worktree = crate::infra::paths::expand_home(worktree)
        .to_string_lossy()
        .to_string();
    let Some(command) = ide::open_command(ctx.settings.ide.as_deref(), &worktree) else {
        return Err("ide is not set: put your editor command in the config's ide key".to_string());
    };
    if dry_run {
        println!("{command}");
        return Ok(());
    }
    terminal::run_shell(&command)?;
    println!("opened {worktree}");
    Ok(())
}

/// Name the tab this process is sitting in. The hub calls it on itself at startup; nothing
/// else needs it, because a spawned tab is named at spawn time.
pub fn set_title(repo_arg: Option<&str>, title: &str, dry_run: bool) -> Result<(), String> {
    let settings = settings_for(repo_arg);
    let title = dash_is_stdin(title)?;
    let done = terminal::set_title(&settings.terminal, &title, dry_run)?;
    if dry_run {
        println!("{}", done.script);
    } else {
        println!("{}", done.description);
    }
    Ok(())
}

pub fn notify_user(
    repo_arg: Option<&str>,
    title: &str,
    message: &str,
    dry_run: bool,
) -> Result<(), String> {
    // Resolved once rather than left to `settings_for`, because a `{nwo}` template needs the
    // same answer the config was picked with — and because this command is the one that can
    // legitimately be run from outside a repository, where there is no answer at all.
    let nwo = match repo_arg {
        Some(arg) => Some(arg.to_string()),
        None => identity::resolve(None, None).ok().map(|info| info.nwo),
    };
    let settings = settings_for(nwo.as_deref());
    if nwo.is_none() && notify::needs_repo(&settings.notification) {
        eprintln!(
            "adjutant: the notification template asks for {{nwo}} but this is not a repository — pass --repo owner/name"
        );
    }
    let Some(command) = notify::command(
        &settings.notification,
        nwo.as_deref().unwrap_or_default(),
        title,
        message,
    ) else {
        // No notifier is a fact about the machine, not a failure of the thing being
        // announced. Say it on stderr and carry on.
        eprintln!("adjutant: no notifier is configured ({title}: {message})");
        return Ok(());
    };
    if dry_run {
        println!("{command}");
        return Ok(());
    }
    terminal::run_shell(&command)?;
    Ok(())
}

// ── worktree ─────────────────────────────────────────────────────────

pub struct WorktreeArgs<'a> {
    pub repo: Option<&'a str>,
    /// A branch you already know. Answers the path only.
    pub branch: Option<&'a str>,
    /// A task name (`app-1234`). Answers the branch *and* the path, as JSON — the two are
    /// always needed together, and deriving them in two places is how they drift apart.
    pub name: Option<&'a str>,
    pub user: Option<&'a str>,
    /// The selected task source's `branchPattern`, when it has one.
    pub pattern: Option<&'a str>,
    /// With `name`: the first of `name`, `name-2`, `name-3`… that nothing holds yet, for a
    /// caller that picks the name itself and so has no one to tell it is taken.
    pub unique: bool,
}

/// Whether a worktree of this name could be created now: its path is free, no local branch
/// has its name, and git does not list a worktree there (a listed one whose directory is gone
/// still holds the branch it had checked out).
fn worktree_name_free(
    main: &str,
    layout: &str,
    pattern: &str,
    user: &str,
    name: &str,
    listed: &[String],
) -> Result<bool, String> {
    let branch = identity::branch_fallback(pattern, user, name);
    let path = identity::worktree_fallback(layout, main, &branch)?;
    if std::path::Path::new(&path).exists() || listed.contains(&path) {
        return Ok(false);
    }
    let taken = crate::infra::git::git(
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
        Some(std::path::Path::new(main)),
    )?;
    Ok(!taken.status.success())
}

pub fn worktree_path(args: &WorktreeArgs<'_>) -> Result<(), String> {
    let ctx = context_without_hub(args.repo)?;
    let layout = ctx
        .settings
        .worktree_pattern
        .as_deref()
        .unwrap_or(identity::DEFAULT_WORKTREE_PATTERN);

    if let Some(branch) = args.branch {
        println!(
            "{}",
            identity::worktree_fallback(layout, &ctx.repo.main, branch)?
        );
        return Ok(());
    }
    let Some(name) = args.name else {
        return Err("pass either --branch or --name".to_string());
    };
    let user = match args.user {
        Some(user) => user.to_string(),
        // Not guessed from the remote: the branch prefix people use is their forge login,
        // which is not always the local account name — so the caller passes it, and this is
        // only the last resort.
        None => std::env::var("USER").unwrap_or_else(|_| "worker".to_string()),
    };
    let pattern = args.pattern.unwrap_or(identity::DEFAULT_BRANCH_PATTERN);
    let name = match args.unique {
        true => {
            let listed = identity::linked_worktrees(&ctx.repo.main)?;
            let mut candidates =
                std::iter::once(name.to_string()).chain((2..1000).map(|n| format!("{name}-{n}")));
            loop {
                let Some(candidate) = candidates.next() else {
                    return Err(format!("no free worktree name starting from {name}"));
                };
                if worktree_name_free(&ctx.repo.main, layout, pattern, &user, &candidate, &listed)?
                {
                    break candidate;
                }
            }
        }
        false => name.to_string(),
    };
    let branch = identity::branch_fallback(pattern, &user, &name);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "name": name,
            "branch": branch,
            "path": identity::worktree_fallback(layout, &ctx.repo.main, &branch)?,
            // Named for what it is: the checkout to run `git worktree add` *in*. It was
            // called `base`, and the procedure duly passed it where git wants a commit-ish
            // — which is a path, so every worktree creation failed with `fatal: invalid
            // reference`. What to branch *from* is the `baseBranch` rule, and that is a
            // question about the repository's branches rather than about this path.
            "main": ctx.repo.main,
        }))
        .unwrap_or_default()
    );
    Ok(())
}
