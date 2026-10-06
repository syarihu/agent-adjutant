use super::*;

/// How many waiting messages `hubs[].inbox` lists, newest first. The count is the whole inbox.
const INBOX_LISTED: usize = 20;

/// All hubs belonging to `repo`, repository hub first, followed by any parent-task hubs.
///
/// A parent-task hub is listed while something points at it: a hub record, or a checkout
/// whose worker reports to it (`children`, counted by slug, so `WID-957` and `wid-957` are
/// one hub). A saved hub session alone does not list it, so a stopped hub whose last
/// checkout is gone leaves the list; its session stays for `--resume`.
pub fn all_repo_hubs(root: &Path, repo: &crate::kernel::identity::RepoInfo) -> Vec<RepoHub> {
    let worktrees = crate::kernel::identity::linked_worktrees(&repo.main).unwrap_or_default();
    all_repo_hubs_among(root, repo, &worktrees)
}

/// `all_repo_hubs` for a caller that has already listed the linked worktrees of `repo`, so that
/// git is not asked for them a second time.
pub fn all_repo_hubs_among(
    root: &Path,
    repo: &crate::kernel::identity::RepoInfo,
    worktrees: &[String],
) -> Vec<RepoHub> {
    all_repo_hubs_among_with(root, &ProcessTable::each(), repo, worktrees)
}

/// `all_repo_hubs_among`, asking `table` when each hub's process started.
pub fn all_repo_hubs_among_with(
    root: &Path,
    table: &ProcessTable,
    repo: &crate::kernel::identity::RepoInfo,
    worktrees: &[String],
) -> Vec<RepoHub> {
    use std::collections::HashMap;

    let (default_slug, default_hub_name) = match &repo.hub {
        Some(_) => {
            let default_repo = repo
                .clone()
                .addressed(None)
                .unwrap_or_else(|_| repo.clone());
            (default_repo.slug, default_repo.hub_name)
        }
        None => (repo.slug.clone(), repo.hub_name.clone()),
    };

    let mut hubs_by_slug: HashMap<String, (Option<String>, String)> = HashMap::new();
    // 1. Always include the repository default hub itself.
    hubs_by_slug.insert(default_slug.clone(), (None, default_hub_name));

    // If the repo context explicitly addresses a parent hub, ensure it is also included.
    if let Some(hub_key) = &repo.hub {
        hubs_by_slug.insert(
            repo.slug.clone(),
            (Some(hub_key.clone()), repo.hub_name.clone()),
        );
    }

    // 2. Discover from state_dir/hubs
    for (slug, record) in hub_records(root) {
        let cwd_matches = record.cwd.as_deref().is_some_and(|cwd| {
            cwd == repo.main
                || matches!(
                    (Path::new(cwd).canonicalize(), Path::new(&repo.main).canonicalize()),
                    (Ok(a), Ok(b)) if a == b
                )
        });
        if !cwd_matches {
            continue;
        }
        let hub_name = record
            .hub_name
            .clone()
            .unwrap_or_else(|| format!("{}{}", crate::kernel::identity::HUB_PREFIX, slug));
        let key = record.hub.clone();
        let entry = hubs_by_slug.entry(slug).or_insert((key.clone(), hub_name));
        if entry.0.is_none() && key.is_some() {
            entry.0 = key;
        }
    }

    // 3. Discover from linked worktrees and the main checkout, counting the ones each hub
    // has. A checkout counts for the hub its worker reports to: the record's, and failing
    // that the saved session's, which is what keeps counting after the worker has ended.
    let mut children: HashMap<String, usize> = HashMap::new();
    let mut checkouts = vec![repo.main.clone()];
    checkouts.extend(worktrees.iter().cloned());
    for wt in checkouts {
        let Some(hub_key) = worker_hub_key(Path::new(&wt)) else {
            continue;
        };
        let slug = crate::kernel::identity::slug_for(&repo.nwo, Some(&hub_key));
        *children.entry(slug.clone()).or_insert(0) += 1;
        let hub_name = format!("{}{}", crate::kernel::identity::HUB_PREFIX, slug);
        let entry = hubs_by_slug
            .entry(slug)
            .or_insert((Some(hub_key.clone()), hub_name));
        if entry.0.is_none() {
            entry.0 = Some(hub_key);
        }
    }

    // 4. Saved sessions in state_dir/sessions only say more about a hub already listed: a
    // parent-task hub nobody has a record or a checkout for is finished, and stays gone.
    for saved in hub_sessions_for(root, &repo.nwo) {
        let slug = crate::kernel::identity::slug_for(&repo.nwo, saved.hub.as_deref());
        let Some(entry) = hubs_by_slug.get_mut(&slug) else {
            continue;
        };
        if entry.0.is_none() && saved.hub.is_some() {
            entry.0 = saved.hub;
        }
        if let Some(name) = saved.hub_name
            && entry.1 == format!("{}{}", crate::kernel::identity::HUB_PREFIX, slug)
        {
            entry.1 = name;
        }
    }

    // Convert to RepoHub
    let mut result: Vec<RepoHub> = hubs_by_slug
        .into_iter()
        .map(|(slug, (mut key, hub_name))| {
            let parent = slug != default_slug;
            // A record written before it carried the key: the saved session may still say,
            // and failing that the slug itself does, when it can be read back unambiguously.
            if key.is_none() && parent {
                key = hub_session(root, &slug)
                    .and_then(|session| session.hub)
                    .or_else(|| crate::kernel::identity::hub_key_from_slug(&repo.nwo, &slug));
            }
            let status = hub_status_with(root, table, &slug, &hub_name);
            let entries = list(root, &slug);
            let inbox_count = entries.len();
            // Only what calls for waking the hub counts as waiting on it: the hub's own
            // question and needs-user copies, acks and notices are not waiting for a read.
            let (mut unseen, mut seen, mut oldest_unseen_at) = (0, 0, None::<String>);
            for entry in &entries {
                if !should_wake_hub(&entry.from, &hub_name, &entry.kind, &entry.subject) {
                    continue;
                }
                if entry.seen {
                    seen += 1;
                    continue;
                }
                unseen += 1;
                if let Some(at) = &entry.at
                    && oldest_unseen_at.as_ref().is_none_or(|oldest| at < oldest)
                {
                    oldest_unseen_at = Some(at.clone());
                }
            }
            // `list` is oldest first, so the newest are at the end.
            let inbox = entries
                .into_iter()
                .rev()
                .take(INBOX_LISTED)
                .map(|entry| InboxItem {
                    counted: should_wake_hub(&entry.from, &hub_name, &entry.kind, &entry.subject),
                    name: entry.name,
                    subject: entry.subject,
                    kind: entry.kind,
                    from: entry.from,
                    worktree: entry.worktree,
                    at: entry.at,
                    seen: entry.seen,
                })
                .collect();
            let id = match &key {
                Some(k) => format!("hub-{}", k.trim()),
                None if slug == default_slug => "hub".to_string(),
                None => format!("hub-{slug}"),
            };
            let children = children.get(&slug).copied().unwrap_or(0);
            RepoHub {
                id,
                parent,
                key,
                name: hub_name,
                title: None,
                slug,
                state: RepoHubState {
                    present: status.present,
                    stale: status.stale,
                    pid: status.pid,
                    started_at: status.started_at,
                },
                inbox_count,
                unseen,
                seen,
                oldest_unseen_at,
                inbox,
                children,
            }
        })
        .collect();

    // Repository default hub first, then sorted by id
    result.sort_by(|a, b| {
        let a_is_repo = a.slug == default_slug;
        let b_is_repo = b.slug == default_slug;
        match (a_is_repo, b_is_repo) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.id.cmp(&b.id),
        }
    });

    result
}
