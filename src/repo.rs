//! Where we are: the main checkout, the repo's canonical `owner/name`, and the hub name
//! derived from it.
//!
//! The hub name IS the address a worker sends its report to, so the derivation lives here
//! and nowhere else. A rule spelled out in the hub's prompt and again in the worker's is a
//! rule with two versions, and the day they disagree the report goes silently nowhere.
//!
//! `owner/name` was doing three jobs at once: the key the config is looked up under, the
//! address the hub answers at, and the thing a worker derives both from. Asking for a second
//! hub therefore meant inventing a second repository name — which moved the address *and*
//! emptied the configuration, because there is no such repository in the config. The hub
//! identifier here splits the second job off the first: it goes into the address and nowhere
//! near the lookup, so `owner/name` keeps answering for the settings while the hub can be
//! named separately.

use std::path::Path;

use crate::infra::git::{GitOut, git_until};
pub(crate) use crate::infra::git::{REPOSITORY_LOCATION_ENV, git};

/// Prefix for every hub session name. `adjutant-` is distinctive enough that a listing can
/// pick hubs out of a pile of worker sessions by prefix alone.
pub const HUB_PREFIX: &str = "adjutant-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// Absolute path of the main checkout (never a linked worktree).
    pub main: String,
    /// `owner/name`, or a bare directory name when there is no usable remote.
    pub nwo: String,
    /// Just the `name` half.
    pub repo: String,
    /// Which hub of this repository, when it is not the repository's own one.
    ///
    /// `None` is the whole-repository hub, and its address is bit for bit the one every
    /// record and inbox already on the machine is filed under — so an upgrade does not
    /// orphan a running hub. Held as it was typed rather than folded: `adj work` forwards
    /// it to the tab it opens, and a person reading that command line should see what they
    /// wrote.
    pub hub: Option<String>,
    pub slug: String,
    pub hub_name: String,
    /// `origin`, `argument`, or `dirname` — how `nwo` was arrived at. The commands warn on
    /// `dirname` because that is the one case where two machines can disagree.
    pub nwo_source: &'static str,
}

/// The worktree this command is being run in — the linked one, not the main checkout.
///
/// The other half of `main_worktree`, and the two are wanted for opposite reasons.
/// A hub launched anywhere in a repository has to land in the main checkout, so that
/// answers with the main one. A message has to say where it came *from*, and "the main
/// checkout" is the one answer that never identifies a worker.
///
/// `None` rather than an error: this is asked on the way to sending a message, and a
/// message sent from outside a repository is still a message. What it costs is the header,
/// and the reader can see it is missing.
pub fn current_worktree(start: Option<&Path>) -> Option<String> {
    let out = git(&["rev-parse", "--show-toplevel"], start).ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // A bare repository answers with nothing at all rather than failing.
    (!path.is_empty()).then_some(path)
}

/// The main checkout. `git worktree list` always prints it first, so this answers the same
/// way from inside a worktree — which is where the hub is usually launched from.
pub fn main_worktree(start: Option<&Path>) -> Result<String, String> {
    let out = git(&["worktree", "list", "--porcelain"], start)?;
    if !out.status.success() {
        return Err("run this inside a git repository".to_string());
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if Path::new(path).is_dir() {
                return Ok(path.to_string());
            }
            return Err(format!("cannot locate the main checkout ({path})"));
        }
    }
    Err("cannot locate the main checkout".to_string())
}

/// The worktrees hanging off `main`, the main checkout itself left out, as absolute paths.
///
/// Left out because the main checkout is where the hub sits rather than a worker. A caller
/// that has to account for a worker put there anyway adds it back itself.
///
/// An error when git cannot answer, rather than an empty list: the question is "which of
/// these is busy", and a list that failed to come back read as "none" would let a dispatch
/// past `maxWorkers` whenever git hiccupped.
pub fn linked_worktrees(main: &str) -> Result<Vec<String>, String> {
    Ok(worktrees(main)?
        .into_iter()
        .filter(|w| Path::new(&w.path) != Path::new(main))
        .map(|w| w.path)
        .collect())
}

/// A worktree as `git worktree list` names it: where it is and what it has checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    /// `None` for a detached HEAD, and for a bare entry.
    pub branch: Option<String>,
}

/// Every worktree of `main`, the main checkout included, with the branch each has checked out.
///
/// An error when git cannot answer, for the reason `linked_worktrees` gives.
pub fn worktrees(main: &str) -> Result<Vec<Worktree>, String> {
    let out = git(&["worktree", "list", "--porcelain"], Some(Path::new(main)))?;
    if !out.status.success() {
        return Err(format!(
            "cannot list the worktrees of {main}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_worktrees(&String::from_utf8_lossy(&out.stdout)))
}

/// The entries of `git worktree list --porcelain`: blocks of `key value` lines, each opening
/// with `worktree <path>` and separated by a blank line.
///
/// Only `worktree` and `branch` are read. `locked` and `prunable` (with or without a reason),
/// `bare`, `detached` and `HEAD` say nothing a caller of this needs, and a path is the rest
/// of its line, so one with spaces is whole.
pub fn parse_worktrees(porcelain: &str) -> Vec<Worktree> {
    let mut found: Vec<Worktree> = Vec::new();
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            found.push(Worktree {
                path: path.to_string(),
                branch: None,
            });
        } else if let Some(branch) = line.strip_prefix("branch ")
            && let Some(last) = found.last_mut()
        {
            last.branch = Some(
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch)
                    .to_string(),
            )
            .filter(|b| !b.is_empty());
        }
    }
    found
}

/// `owner/name` from a remote URL.
///
/// Taking the *last two* path segments makes `git@host:o/r`, `https://host/o/r`,
/// `ssh://host/o/r` and `git://host/o/r` all agree. Stripping to the last separator instead
/// breaks on `ssh://`, which has one more.
pub fn nwo_from_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let parts: Vec<&str> = url.split(['/', ':']).filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        Some(format!(
            "{}/{}",
            parts[parts.len() - 2],
            parts[parts.len() - 1]
        ))
    } else {
        None
    }
}

/// The host a remote URL names, without a user or a port: `github.com` out of
/// `git@github.com:o/r.git`, `https://github.com/o/r` and `ssh://git@github.com:22/o/r`.
pub fn host_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let after_scheme = url.split_once("://").map(|(_, rest)| rest);
    let authority = match after_scheme {
        Some(rest) => rest.split('/').next()?,
        // scp-like: `user@host:path`
        None => url.split_once(':')?.0,
    };
    let host = authority.rsplit('@').next()?;
    let host = if after_scheme.is_some() {
        host.split(':').next()?
    } else {
        host
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The host origin points at, taken offline like `name_with_owner`.
pub fn origin_host(main: &str) -> Option<String> {
    let out = git(&["-C", main, "remote", "get-url", "origin"], None)
        .ok()
        .filter(|o| o.status.success())?;
    host_from_url(String::from_utf8_lossy(&out.stdout).trim())
}

/// `owner/name` from origin, taken offline so a renamed directory cannot change it.
///
/// Keeping the owner matters: `orgA/app` and `orgB/app` would otherwise share a hub name,
/// and launching the second one would find the first and refuse as a duplicate.
pub fn name_with_owner(main: &str) -> (String, &'static str) {
    let url = git(&["-C", main, "remote", "get-url", "origin"], None)
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    match nwo_from_url(&url) {
        Some(nwo) => (nwo, "origin"),
        None => (
            Path::new(main)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| main.to_string()),
            "dirname",
        ),
    }
}

/// Lowercase, then `.` `_` `/` collapse to `-`, then a digest of the name it came from.
/// Both the side that names the hub and the side that looks for it call this, rather than
/// each spelling the rule out.
///
/// The readable half is a courtesy — it is what a person sees in `ps` and in the state
/// directory. The digest is what makes the answer an *address*. Collapsing throws
/// information away, and every repository whose name collapses to the same characters was
/// sharing one inbox: `acme/foo-bar`, `acme/foo_bar` and `acme/foo.bar` are one slug, so is
/// `acme-foo/bar`, and every name with no ASCII in it collapsed to the same bare prefix. A
/// report filed against one arrived at the other, and nothing anywhere said so.
///
/// Appending it unconditionally rather than only when the collapse looked lossy: every
/// conditional rule leaks a case, and a rule with an exception is one both sides have to
/// agree about exactly.
///
/// `None` means the repository's own hub and takes the branch this has always taken, digest
/// and all, because there are hub records and inboxes on disk under those names right now:
/// a slug that changed shape on upgrade would leave every running hub unreachable and every
/// queued report unread.
///
/// With an identifier, the digest is taken of *both* halves. Digesting the repository alone
/// and appending the identifier to the readable half would put `acme/widget` + `foo-bar` and
/// `acme/widget` + `foo.bar` back in one inbox, which is the collapse the digest exists to
/// undo. The two are joined by a newline, which neither can contain — a repository name
/// comes out of a remote URL and a hub identifier is a word somebody typed — so no pair can
/// borrow another pair's address.
pub fn slug_for(nwo: &str, hub: Option<&str>) -> String {
    let base = readable(nwo);
    // Nothing readable left is still an error at `hub_name`, where it can say so. A slug
    // that is only a digest names nothing a person could recognise.
    if base.is_empty() {
        return String::new();
    }
    // Of the *lowercased* name, so that the address is as case-insensitive as the config
    // lookup beside it. Hashing the original made `Acme/Widget` and `acme/widget` two
    // addresses for one registered repository — a hub started from one and a worker from
    // the other, each writing to an inbox the other never reads.
    let Some(hub) = hub.filter(|hub| !hub.is_empty()) else {
        return format!("{base}-{:016x}", fnv1a(&nwo.to_lowercase()));
    };
    let digest = fnv1a(&format!("{}\n{}", nwo.to_lowercase(), hub.to_lowercase()));
    // An identifier with no ASCII in it contributes nothing a person could read, and a
    // trailing `-` before the digest would only look like a typo. It is still in the
    // digest, which is the half that makes the slug an address.
    match readable(hub) {
        tail if tail.is_empty() => format!("{base}-{digest:016x}"),
        tail => format!("{base}-{tail}-{digest:016x}"),
    }
}

/// The hub identifier a parent-task hub's slug was made from, read back out of the slug.
///
/// For hub records written before they carried the identifier. Only the readable half is
/// there to read, so the answer is that half, accepted only when it hashes back to the same
/// slug: the address is case-insensitive, so a lowercased identifier still names the same
/// hub, but one that lost characters to the collapse would name another. `None` when the
/// slug is the repository's own, or the identifier cannot be told from it.
pub fn hub_key_from_slug(nwo: &str, slug: &str) -> Option<String> {
    let base = readable(nwo);
    let rest = slug.strip_prefix(&base)?.strip_prefix('-')?;
    let (tail, digest) = rest.rsplit_once('-')?;
    if digest.len() != 16 || tail.is_empty() {
        return None;
    }
    (slug_for(nwo, Some(tail)) == slug).then(|| tail.to_string())
}

/// The half of a slug a person recognises: lowercase, `.` `_` `/` collapsed to `-`, and
/// everything else dropped.
fn readable(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            '.' | '_' | '/' => '-',
            c => c,
        })
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect()
}

/// FNV-1a, written out rather than taken from the standard library: `DefaultHasher`'s
/// algorithm is explicitly not stable between Rust releases, and a slug that changes when
/// the binary is rebuilt re-addresses every inbox on the machine.
///
/// 64 bits rather than 32. The population that has to stay distinct is not "repositories on
/// this machine" but "names that collapse to the same readable half", and a 32-bit digest
/// over that is small enough to collide on purpose — two names collapsing to `acme-` and
/// then to one digest were found by search, not by luck.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The first eight hex digits of `fnv1a(text)`: short enough to read in an id, stable for the
/// reason `fnv1a` is.
pub(crate) fn short_digest(text: &str) -> String {
    format!("{:016x}", fnv1a(text))[..8].to_string()
}

pub fn hub_name(nwo: &str, hub: Option<&str>) -> Result<String, String> {
    let slug = slug_for(nwo, hub);
    if slug.is_empty() {
        return Err(format!(
            "cannot build a hub name from the repository name ({nwo})"
        ));
    }
    Ok(format!("{HUB_PREFIX}{slug}"))
}

pub fn resolve(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<RepoInfo, String> {
    resolve_in(None, repo_arg, hub_arg)
}

/// Same, but answering for `start` rather than for the process's own directory. An MCP
/// server is started once per session and then asked about whichever checkout the caller is
/// sitting in, which is not necessarily the one it was launched from.
pub fn resolve_in(
    start: Option<&Path>,
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
) -> Result<RepoInfo, String> {
    let main = main_worktree(start)?;
    let (nwo, source) = match repo_arg {
        Some(arg) => (arg.to_string(), "argument"),
        None => name_with_owner(&main),
    };
    repo_info(main, nwo, source, hub_arg)
}

impl RepoInfo {
    /// The same repository, at the address of hub `hub_arg` — the repository's own hub when
    /// that is `None`, whatever this one was addressed at.
    ///
    /// Separate from `resolve` because the identifier is not always known before the
    /// repository is: `agentEnv` can name one, and it is looked up under `owner/name`.
    pub fn addressed(self, hub_arg: Option<&str>) -> Result<RepoInfo, String> {
        repo_info(self.main, self.nwo, self.nwo_source, hub_arg)
    }
}

fn repo_info(
    main: String,
    nwo: String,
    source: &'static str,
    hub_arg: Option<&str>,
) -> Result<RepoInfo, String> {
    // Blank is absent, here as well as at every gate in front of this one: an identifier
    // that is only whitespace would otherwise be a hub that exists, has an address nobody
    // can type twice, and is invisible in every listing.
    let hub = hub_arg
        .map(str::trim)
        .filter(|hub| !hub.is_empty())
        .map(str::to_string);
    let hub_name = hub_name(&nwo, hub.as_deref())?;
    Ok(RepoInfo {
        repo: nwo.rsplit('/').next().unwrap_or(&nwo).to_string(),
        slug: slug_for(&nwo, hub.as_deref()),
        hub_name,
        hub,
        main,
        nwo,
        nwo_source: source,
    })
}

/// Where a worktree for `branch` would live when nothing else says otherwise.
///
/// A convention tool owns this when one is installed; these are the answers for a machine
/// without one, so that worktree work still completes standalone. They deliberately match
/// the shape such a tool uses rather than inventing a second one: two tools disagreeing
/// about where a worktree lives is worse than either answer on its own, and the procedure's
/// own "am I in a worktree" check looks for this path.
///
/// Placeholders: `{repo}`, `{branch}`, and `{name}` (the branch with slashes flattened).
pub const DEFAULT_WORKTREE_PATTERN: &str = ".claude/worktrees/{name}";

/// The branch a task's worktree sits on. Placeholders: `{user}`, `{name}`.
///
/// `{name}` is the task's own name (`app-1234`), which is what keeps a repo drawing from two
/// trackers from colliding — the key is part of the name, not of the pattern.
pub const DEFAULT_BRANCH_PATTERN: &str = "{user}/{name}";

/// The branch name for a task called `name`, owned by `user`.
pub fn branch_fallback(pattern: &str, user: &str, name: &str) -> String {
    pattern.replace("{user}", user).replace("{name}", name)
}

/// A branch name that cannot be used to walk out of the checkout.
///
/// `{name}` flattens slashes, so whatever it holds can only ever name one directory.
/// `{branch}` keeps them on purpose — `someone/app-1234` is meant to nest — which is the
/// same property that lets `../../..` through. Git will not put `..` in a ref, so a branch
/// carrying one did not come from a branch: it came from an argument or a pattern, and
/// handing back a path outside the repository is not a better answer than refusing.
fn check_branch(branch: &str) -> Result<(), String> {
    if branch.trim().is_empty() {
        return Err("the branch name is empty".to_string());
    }
    if branch.starts_with('/') {
        return Err(format!(
            "branch name {branch} starts with a separator: it names a path, not a branch"
        ));
    }
    if branch.split('/').any(|part| part == "." || part == "..") {
        return Err(format!(
            "branch name {branch} walks out of the checkout: it cannot be used as a path"
        ));
    }
    Ok(())
}

pub fn worktree_fallback(pattern: &str, main: &str, branch: &str) -> Result<String, String> {
    check_branch(branch)?;
    let repo = Path::new(main)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| main.to_string());
    let name = branch.replace('/', "-");
    let rendered = pattern
        .replace("{repo}", &repo)
        .replace("{branch}", branch)
        .replace("{name}", &name);
    let path = Path::new(&rendered);
    if path.is_absolute() {
        return Ok(rendered);
    }
    // Relative patterns are relative to the main checkout, not to the caller's cwd — the
    // caller is usually a worktree somewhere else entirely.
    Ok(normalise(&Path::new(main).join(path)))
}

/// Collapse `.` and `..` lexically. `../{repo}-{name}` next to the main checkout is the
/// whole point of the fallback, and `fs::canonicalize` cannot help: the directory does not
/// exist yet at the moment we are computing where to put it.
fn normalise(path: &Path) -> String {
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if out.len() > 1 {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str().to_os_string()),
        }
    }
    let mut buf = std::path::PathBuf::new();
    for part in out {
        buf.push(part);
    }
    buf.to_string_lossy().to_string()
}

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
