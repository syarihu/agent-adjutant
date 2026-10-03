//! What a worktree has not yet let go of.

use std::path::Path;

use crate::infra::git::{GitOut, git_until};

// ── what a worktree has not yet let go of ────────────────────────────

/// Files and lines in a worktree that no commit has yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Uncommitted {
    /// Tracked files that differ from HEAD, staged or not.
    pub files: usize,
    /// Entries git does not track and does not ignore, counted apart: a build directory nobody
    /// ignored is not work at risk in the way an edit is. An entry, not a file: a wholly new
    /// directory counts once, and the lines of untracked files are not in `insertions`.
    pub untracked: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UnpushedCommit {
    pub sha: String,
    pub subject: String,
}

/// Commits that exist only here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unpushed {
    pub count: usize,
    /// The newest of them, up to `UNPUSHED_LISTED`.
    pub commits: Vec<UnpushedCommit>,
    /// What they were counted against: "upstream" (`@{u}..HEAD`) or, for a branch with none,
    /// "remotes" (every commit no remote-tracking ref has).
    pub against: &'static str,
}

/// Whether HEAD is already in the base branch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedInto {
    pub base: Option<String>,
    /// The ref HEAD was compared with (`origin/main`, or a local branch).
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    /// `None` when it could not be told; `reason` says why.
    pub merged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What a worktree holds that would be lost, or has to be published, if it were removed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitState {
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The short id of HEAD; `None` before the first commit.
    pub head: Option<String>,
    pub uncommitted: Uncommitted,
    pub upstream: Option<String>,
    pub unpushed: Unpushed,
    pub merged: MergedInto,
}

const UNPUSHED_LISTED: usize = 20;

/// Keeps a look from starting a file-system monitor daemon or running its hook in a worktree
/// that is not ours.
const NO_FSMONITOR: &[&str] = &["-c", "core.fsmonitor=false"];

/// Pathspecs that leave out what adjutant itself writes into a worktree: the worker's record,
/// outbox, saved session and starting marker, and the brief. They are bookkeeping, not work,
/// and in a repository that does not ignore `.claude/` they would make every worker's
/// worktree look dirty.
const WORKTREE_ONLY: &[&str] = &[
    "--",
    ".",
    ":(exclude).claude/adjutant-*",
    ":(exclude).claude/task-brief.md",
];

/// What `worktree` holds that no remote has, for a person deciding whether it can go.
///
/// `Ok(None)` when the directory is gone. `base` is the branch the work is meant to land on
/// (a task's), and without one the remote's default. Nothing is fetched: "merged" and "pushed"
/// are as of the last fetch, and a squash or rebase merge is not seen as one, because the
/// commits it left in the base are not the ones here.
pub fn worktree_git_state(
    worktree: &Path,
    base: Option<&str>,
    deadline: std::time::Instant,
) -> Result<Option<GitState>, String> {
    if !worktree.is_dir() {
        return Ok(None);
    }
    let git = |args: &[&str]| git_until(args, worktree, deadline);
    let line =
        |out: GitOut| Some(out.stdout.trim().to_string()).filter(|l| out.ok() && !l.is_empty());

    // Exit 1 is the answer "no": a detached HEAD has no branch, an unborn one no commit.
    // Any other failure is git not answering, and is not read as either.
    let branch = git(&["symbolic-ref", "-q", "--short", "HEAD"])?;
    let branch = match branch.code {
        Some(0) => line(branch),
        Some(1) => None,
        _ => return Err("git could not read the branch".to_string()),
    };
    let head = git(&["rev-parse", "-q", "--verify", "--short", "HEAD"])?;
    let head = match head.code {
        Some(0) => line(head),
        Some(1) if head.stdout.trim().is_empty() => None,
        _ => return Err("git could not read HEAD".to_string()),
    };

    let status = git(&[
        NO_FSMONITOR,
        &["status", "--porcelain=v1", "-z"],
        WORKTREE_ONLY,
    ]
    .concat())?;
    if !status.ok() {
        return Err(format!("{} is not a git worktree", worktree.display()));
    }
    let mut uncommitted = Uncommitted::default();
    let mut fields = status.stdout.split('\0').filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        if field.starts_with("??") {
            uncommitted.untracked += 1;
            continue;
        }
        uncommitted.files += 1;
        // A rename or copy is followed by the path it came from, a field of its own.
        if field.starts_with(['R', 'C']) || field.get(1..2).is_some_and(|y| y == "R" || y == "C") {
            fields.next();
        }
    }
    if head.is_some() {
        let numstat = git(&[NO_FSMONITOR, &["diff", "--numstat", "HEAD"], WORKTREE_ONLY].concat())?;
        if !numstat.ok() {
            return Err("git could not count the changed lines".to_string());
        }
        for row in numstat.stdout.lines() {
            let mut cols = row.split('\t');
            // `-` stands in for both counts of a binary file.
            uncommitted.insertions += cols.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            uncommitted.deletions += cols.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }

    // Only a branch that has commits can have an upstream. `config` exits 1 for a key that is
    // not there, which is the one answer that means "none"; any other failure, and a git that
    // did not answer in time, is an error rather than a branch with nothing to push.
    let mut upstream = None;
    let mut tracks_own_name = false;
    if let (Some(_), Some(name)) = (&head, &branch) {
        let merge = git(&["config", "--get", &format!("branch.{name}.merge")])?;
        match merge.code {
            Some(0) => {
                // Absent when the remote-tracking branch it names is gone, which is a branch
                // whose upstream was deleted, not one that is unreadable.
                upstream = line(git(&["rev-parse", "--abbrev-ref", "@{u}"])?);
                tracks_own_name = merge.stdout.trim().strip_prefix("refs/heads/") == Some(name);
            }
            Some(1) => {}
            _ => return Err("git could not read the branch's upstream".to_string()),
        }
    }
    // Counted against the upstream only when it is the branch's own counterpart. A worktree
    // made with `worktree add -b x origin/main` has `origin/main` as its upstream, and against
    // that every commit not yet in main would count, and none would once it is merged.
    let against_upstream = upstream.is_some() && tracks_own_name;
    let mut unpushed = Unpushed {
        count: 0,
        commits: Vec::new(),
        against: if against_upstream {
            "upstream"
        } else {
            "remotes"
        },
    };
    if head.is_some() {
        let range: &[&str] = match against_upstream {
            true => &["@{u}..HEAD"],
            false => &["HEAD", "--not", "--remotes"],
        };
        let mut count = vec!["rev-list", "--count"];
        count.extend(range);
        let counted = git(&count)?;
        unpushed.count = counted
            .ok()
            .then(|| counted.stdout.trim().parse().ok())
            .flatten()
            .ok_or("git could not count the unpushed commits")?;
        if unpushed.count > 0 {
            let limit = format!("-n{UNPUSHED_LISTED}");
            let mut log = vec!["log", "--format=%h%x09%s", &limit];
            log.extend(range);
            let listed = git(&log)?;
            if !listed.ok() {
                return Err("git could not list the unpushed commits".to_string());
            }
            unpushed.commits = listed
                .stdout
                .lines()
                .filter_map(|row| row.split_once('\t'))
                .map(|(sha, subject)| UnpushedCommit {
                    sha: sha.to_string(),
                    subject: subject.to_string(),
                })
                .collect();
        }
    }

    let merged = merged_into(worktree, head.is_some(), base, deadline)?;
    Ok(Some(GitState {
        branch,
        head,
        uncommitted,
        upstream,
        unpushed,
        merged,
    }))
}

fn merged_into(
    worktree: &Path,
    has_head: bool,
    base: Option<&str>,
    deadline: std::time::Instant,
) -> Result<MergedInto, String> {
    let git = |args: &[&str]| git_until(args, worktree, deadline);
    let told = |base: Option<String>, reference: Option<String>, reason: &str| MergedInto {
        base,
        reference,
        merged: None,
        reason: Some(reason.to_string()),
    };
    let named = base
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_string);
    // Without a base of its own, the remote's default; and where a clone never set
    // `origin/HEAD`, the two names nearly every repository's default has.
    let mut candidates = Vec::new();
    match &named {
        Some(name) => candidates.push(name.clone()),
        None => {
            let out = git(&["symbolic-ref", "-q", "--short", "refs/remotes/origin/HEAD"])?;
            if out.ok() && !out.stdout.trim().is_empty() {
                candidates.push(out.stdout.trim().to_string());
            }
            candidates.extend(["main".to_string(), "master".to_string()]);
        }
    }
    let candidates: Vec<String> = candidates
        .into_iter()
        .map(|c| c.strip_prefix("origin/").unwrap_or(&c).to_string())
        .fold(Vec::new(), |mut all, c| {
            if !all.contains(&c) {
                all.push(c);
            }
            all
        });
    if !has_head {
        return Ok(told(
            candidates.first().cloned().filter(|_| named.is_some()),
            None,
            "no commit yet",
        ));
    }
    for base in &candidates {
        // The remote-tracking branch first: a local one that was never updated says "not
        // merged" about work the remote already has.
        for (full, shown) in [
            (
                format!("refs/remotes/origin/{base}"),
                format!("origin/{base}"),
            ),
            (format!("refs/heads/{base}"), base.clone()),
        ] {
            let probe = format!("{full}^{{commit}}");
            if !git(&["rev-parse", "--verify", "-q", &probe])?.ok() {
                continue;
            }
            let out = git(&["merge-base", "--is-ancestor", "HEAD", &full])?;
            let merged = match out.code {
                Some(0) => Some(true),
                Some(1) => Some(false),
                _ => None,
            };
            return Ok(MergedInto {
                base: Some(base.clone()),
                reference: Some(shown),
                merged,
                reason: merged
                    .is_none()
                    .then(|| "git could not compare them".to_string()),
            });
        }
    }
    Ok(match named {
        Some(_) => told(
            candidates.first().cloned(),
            None,
            "the base branch is not in this repository",
        ),
        None => told(None, None, "no base branch is known"),
    })
}

#[cfg(test)]
mod tests;
