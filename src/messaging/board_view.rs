use super::*;

/// The hub a checkout's worker reports to: the key its record says, and failing that the one
/// its saved session says. The record wins because `adj worker --hub` rewrites it on a
/// resume, while the session is only written when a session starts.
pub fn worker_hub_key(worktree: &Path) -> Option<String> {
    let clean = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    read_json(&worker_record_path(worktree))
        .and_then(|record| record.get("hub").and_then(Value::as_str).and_then(clean))
        .or_else(|| worker_session(worktree).and_then(|saved| saved.hub.as_deref().and_then(clean)))
}

/// The task a checkout's worker is on: what its record says, and only when it has no record
/// what its saved session says. A record without a task is a session that has none, and a
/// session saved before it was linked must not give it back one it has since been moved off.
pub fn worker_task(worktree: &Path) -> Option<String> {
    let clean = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    match read_json(&worker_record_path(worktree)) {
        Some(record) => record.get("task").and_then(Value::as_str).and_then(clean),
        None => worker_session(worktree).and_then(|saved| saved.task.as_deref().and_then(clean)),
    }
}

/// How many waiting messages `hubs[].inbox` lists, newest first. The count is the whole inbox.
const INBOX_LISTED: usize = 20;

/// All hubs belonging to `repo`, repository hub first, followed by any parent-task hubs.
///
/// A parent-task hub is listed while something points at it: a hub record, or a checkout
/// whose worker reports to it (`children`, counted by slug, so `WID-957` and `wid-957` are
/// one hub). A saved hub session alone does not list it, so a stopped hub whose last
/// checkout is gone leaves the list; its session stays for `--resume`.
pub fn all_repo_hubs(repo: &crate::kernel::identity::RepoInfo) -> Vec<crate::session::RepoHub> {
    let worktrees = crate::kernel::identity::linked_worktrees(&repo.main).unwrap_or_default();
    all_repo_hubs_among(repo, &worktrees)
}

/// `all_repo_hubs` for a caller that has already listed the linked worktrees of `repo`, so that
/// git is not asked for them a second time.
pub fn all_repo_hubs_among(
    repo: &crate::kernel::identity::RepoInfo,
    worktrees: &[String],
) -> Vec<crate::session::RepoHub> {
    all_repo_hubs_among_with(&ProcessTable::each(), repo, worktrees)
}

/// `all_repo_hubs_among`, asking `table` when each hub's process started.
pub fn all_repo_hubs_among_with(
    table: &ProcessTable,
    repo: &crate::kernel::identity::RepoInfo,
    worktrees: &[String],
) -> Vec<crate::session::RepoHub> {
    use crate::session::{InboxItem, RepoHub, RepoHubState};
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
    let hubs_dir = state_dir().join("hubs");
    if let Ok(entries) = std::fs::read_dir(&hubs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json")
                && let Some(record) = read_json(&path)
            {
                let cwd_matches = record
                    .get("cwd")
                    .and_then(Value::as_str)
                    .is_some_and(|cwd| {
                        cwd == repo.main
                            || matches!(
                                (Path::new(cwd).canonicalize(), Path::new(&repo.main).canonicalize()),
                                (Ok(a), Ok(b)) if a == b
                            )
                    });
                let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if cwd_matches && !file_stem.is_empty() {
                    let hub_name = record
                        .get("hubName")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!("{}{}", crate::kernel::identity::HUB_PREFIX, file_stem)
                        });
                    let key = record
                        .get("hub")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let entry = hubs_by_slug
                        .entry(file_stem.to_string())
                        .or_insert((key.clone(), hub_name));
                    if entry.0.is_none() && key.is_some() {
                        entry.0 = key;
                    }
                }
            }
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
    for saved in hub_sessions_for(&repo.nwo) {
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
                key = read_session(&hub_session_path(&slug))
                    .and_then(|session| session.hub)
                    .or_else(|| crate::kernel::identity::hub_key_from_slug(&repo.nwo, &slug));
            }
            let status = hub_status_with(table, &slug, &hub_name);
            let entries = list(&slug);
            let inbox_count = entries.len();
            // `list` is oldest first, so the newest are at the end.
            let inbox = entries
                .into_iter()
                .rev()
                .take(INBOX_LISTED)
                .map(|entry| InboxItem {
                    name: entry.name,
                    subject: entry.subject,
                    kind: entry.kind,
                    from: entry.from,
                    worktree: entry.worktree,
                    at: entry.at,
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
