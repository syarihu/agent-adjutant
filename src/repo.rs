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
use std::process::Command;

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

/// The variables with which git is told which repository to use, ahead of the directory it is
/// started in.
///
/// Every question asked here is about the repository a directory belongs to, so git has to
/// find it from that directory. Git exports `GIT_DIR` to the hooks it runs, and a person can
/// have one set; left in place, any of these would answer for another checkout — its hub
/// name, its inbox, its worker count — with nothing reporting the swap. `GIT_INDEX_FILE`,
/// `GIT_OBJECT_DIRECTORY` and the like stay: they do not choose the repository.
/// The agent `adj hub` and `adj work` start is handed an environment without them too.
pub(crate) const REPOSITORY_LOCATION_ENV: [&str; 3] =
    ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR"];

/// Git, answering for the repository `cwd` (or the current directory) is in.
pub(crate) fn git(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output, String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    for name in REPOSITORY_LOCATION_ENV {
        cmd.env_remove(name);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output().map_err(|e| format!("cannot run git: {e}"))
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

struct GitOut {
    code: Option<i32>,
    stdout: String,
}

impl GitOut {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Git in `cwd`, killed when `deadline` passes.
///
/// Read-only by construction: `GIT_OPTIONAL_LOCKS=0` keeps `status` from refreshing the index
/// behind the back of a worker that is running git in the same worktree. Output is drained on
/// threads, because a listing longer than a pipe holds would otherwise stall the child until
/// the deadline.
///
/// It reads the index and refs of a worktree that is not ours, so beyond the variables that
/// choose the repository it also drops the ones that would swap in another index, object store
/// or ref namespace from whatever environment the server was started in.
fn git_until(args: &[&str], cwd: &Path, deadline: std::time::Instant) -> Result<GitOut, String> {
    use std::io::Read;
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    for name in REPOSITORY_LOCATION_ENV.into_iter().chain([
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ]) {
        cmd.env_remove(name);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot run git: {e}"))?;
    let mut pipe = child.stdout.take().ok_or("cannot read git's output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // Not joined: a grandchild that inherited the pipe keeps it open past the
                // kill, and waiting for its end would hold the deadline hostage. The thread
                // ends with the pipe.
                drop(reader);
                return Err("git did not answer in time".to_string());
            }
            Err(e) => return Err(format!("cannot wait for git: {e}")),
        }
    };
    let bytes = reader.join().unwrap_or_default();
    Ok(GitOut {
        code: status.code(),
        stdout: String::from_utf8_lossy(&bytes).to_string(),
    })
}

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
mod tests {
    use super::*;

    fn run_git(dir: &Path, args: &[&str]) {
        let mut full = vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ];
        full.extend(args);
        let out = git(&full, Some(dir)).unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit_file(dir: &Path, name: &str, text: &str) {
        std::fs::write(dir.join(name), text).unwrap();
        run_git(dir, &["add", name]);
        run_git(dir, &["commit", "-q", "-m", &format!("add {name}")]);
    }

    fn state_of(dir: &Path, base: Option<&str>) -> GitState {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        worktree_git_state(dir, base, deadline).unwrap().unwrap()
    }

    #[test]
    fn a_worktree_that_is_gone_has_no_git_state() {
        let here = tempfile::tempdir().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let gone = here.path().join("nowhere");
        assert_eq!(worktree_git_state(&gone, None, deadline), Ok(None));
    }

    #[test]
    fn an_unborn_head_and_a_detached_one_are_answers_not_errors() {
        let sandbox = crate::testing::Sandbox::empty();
        let _ = &sandbox;
        let here = tempfile::tempdir().unwrap();
        let dir = here.path();
        crate::testing::init_repo(dir, "main");
        let fresh = state_of(dir, None);
        assert_eq!(fresh.head, None);
        assert_eq!(fresh.unpushed.count, 0);

        commit_file(dir, "a.txt", "1\n");
        run_git(dir, &["checkout", "-q", "--detach"]);
        let detached = state_of(dir, None);
        assert_eq!(detached.branch, None);
        assert!(detached.head.is_some());
    }

    #[test]
    fn uncommitted_work_is_counted_by_file_line_and_untracked() {
        let sandbox = crate::testing::Sandbox::empty();
        let _ = &sandbox;
        let here = tempfile::tempdir().unwrap();
        let dir = here.path();
        crate::testing::init_repo(dir, "main");
        commit_file(dir, "a.txt", "one\ntwo\nthree\n");
        commit_file(dir, "gone.txt", "x\ny\n");
        std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\nfour\n").unwrap();
        std::fs::write(dir.join("new.txt"), "untracked\n").unwrap();
        // What adjutant writes into a worktree is not work, even where `.claude/` is not
        // ignored; anything else in there is.
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        for name in [
            "adjutant-worker.json",
            "adjutant-outbox.md",
            "task-brief.md",
        ] {
            std::fs::write(dir.join(".claude").join(name), "x\n").unwrap();
        }
        std::fs::write(dir.join("blob.bin"), [0u8, 159, 146, 150, 0]).unwrap();
        run_git(dir, &["add", "blob.bin"]);
        std::fs::remove_file(dir.join("gone.txt")).unwrap();

        let state = state_of(dir, None);
        assert_eq!(state.branch.as_deref(), Some("main"));
        // a.txt edited, blob.bin staged, gone.txt deleted; new.txt is not tracked.
        assert_eq!(state.uncommitted.files, 3);
        assert_eq!(state.uncommitted.untracked, 1);
        std::fs::write(dir.join(".claude").join("settings.json"), "{}\n").unwrap();
        assert_eq!(state_of(dir, None).uncommitted.untracked, 2);
        // a.txt: one line changed and one added; gone.txt: two lines; the binary counts none.
        assert_eq!(state.uncommitted.insertions, 2);
        assert_eq!(state.uncommitted.deletions, 3);
    }

    #[test]
    fn with_no_upstream_commits_no_remote_has_are_unpushed() {
        let sandbox = crate::testing::Sandbox::empty();
        let _ = &sandbox;
        let here = tempfile::tempdir().unwrap();
        let dir = here.path();
        crate::testing::init_repo(dir, "main");
        commit_file(dir, "a.txt", "1\n");
        commit_file(dir, "b.txt", "2\n");
        let state = state_of(dir, None);
        assert_eq!(state.upstream, None);
        assert_eq!(state.unpushed.against, "remotes");
        assert_eq!(state.unpushed.count, 2);
        assert_eq!(state.unpushed.commits[0].subject, "add b.txt");
        // No `origin/HEAD` and no remote: the local `main` stands in for the base.
        assert_eq!(state.merged.reference.as_deref(), Some("main"));
        assert_eq!(state.merged.merged, Some(true));

        run_git(dir, &["checkout", "-q", "-b", "feature"]);
        commit_file(dir, "c.txt", "3\n");
        assert_eq!(state_of(dir, None).merged.merged, Some(false));
        run_git(dir, &["branch", "-m", "main", "trunk"]);
        let unknown = state_of(dir, None);
        assert_eq!(unknown.merged.merged, None);
        assert_eq!(unknown.merged.base, None);
        assert!(unknown.merged.reason.is_some());
    }

    #[test]
    fn commits_ahead_of_the_upstream_are_counted_and_merging_follows_the_base() {
        let sandbox = crate::testing::Sandbox::empty();
        let _ = &sandbox;
        let root = tempfile::tempdir().unwrap();
        let origin = root.path().join("origin.git");
        let work = root.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        run_git(
            root.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        crate::testing::init_repo(&work, "main");
        run_git(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        commit_file(&work, "a.txt", "1\n");
        run_git(&work, &["push", "-q", "-u", "origin", "main"]);
        run_git(&work, &["remote", "set-head", "origin", "main"]);

        run_git(&work, &["checkout", "-q", "-b", "feature"]);
        commit_file(&work, "b.txt", "2\n");
        run_git(&work, &["push", "-q", "-u", "origin", "feature"]);
        commit_file(&work, "c.txt", "3\n");

        let state = state_of(&work, None);
        assert_eq!(state.branch.as_deref(), Some("feature"));
        assert_eq!(state.upstream.as_deref(), Some("origin/feature"));
        assert_eq!(state.unpushed.against, "upstream");
        assert_eq!(state.unpushed.count, 1);
        assert_eq!(state.unpushed.commits.len(), 1);
        // The default branch comes from origin/HEAD, and the feature is not in it.
        assert_eq!(state.merged.base.as_deref(), Some("main"));
        assert_eq!(state.merged.reference.as_deref(), Some("origin/main"));
        assert_eq!(state.merged.merged, Some(false));

        // `origin/main` names the same base as `main`.
        assert_eq!(
            state_of(&work, Some("origin/main")).merged.base.as_deref(),
            Some("main")
        );

        run_git(&work, &["push", "-q", "origin", "feature:main"]);
        run_git(&work, &["fetch", "-q", "origin"]);
        let landed = state_of(&work, Some("main"));
        assert_eq!(landed.merged.merged, Some(true));
        assert_eq!(landed.merged.reference.as_deref(), Some("origin/main"));

        // A worktree started from `origin/main` has it as its upstream, but what it has not
        // pushed is measured against the remotes, not against main.
        run_git(&work, &["checkout", "-q", "-b", "task", "origin/main"]);
        commit_file(&work, "d.txt", "4\n");
        let task = state_of(&work, None);
        assert_eq!(task.upstream.as_deref(), Some("origin/main"));
        assert_eq!(task.unpushed.against, "remotes");
        assert_eq!(task.unpushed.count, 1);

        // A base that is nowhere in the repository is said, not guessed.
        let unknown = state_of(&work, Some("release"));
        assert_eq!(unknown.merged.merged, None);
        assert!(unknown.merged.reason.is_some());
    }

    #[test]
    fn a_hub_key_is_read_back_from_its_slug_only_when_it_hashes_back() {
        let slug = slug_for("acme/widget", Some("WID-100"));
        // Lowercased, and still the same address.
        assert_eq!(
            hub_key_from_slug("acme/widget", &slug).as_deref(),
            Some("wid-100")
        );
        // `.` collapsed to `-`: the readable half names another hub, so nothing is claimed.
        let lossy = slug_for("acme/widget", Some("v1.2"));
        assert_eq!(hub_key_from_slug("acme/widget", &lossy), None);
        assert_eq!(
            hub_key_from_slug("acme/widget", &slug_for("acme/widget", None)),
            None
        );
        assert_eq!(
            hub_key_from_slug("acme/widget", &slug_for("acme/other", Some("x"))),
            None
        );
    }

    #[test]
    fn parse_worktrees_reads_path_and_branch_of_each_entry() {
        let porcelain = "worktree /repo/main\nHEAD 1111\nbranch refs/heads/main\n\n\
            worktree /repo/wt/with space\nHEAD 2222\nbranch refs/heads/feature/login-form\n\n\
            worktree /repo/wt/detached\nHEAD 3333\ndetached\n\n\
            worktree /repo/wt/locked\nHEAD 4444\nbranch refs/heads/locked-one\nlocked in use\n\n\
            worktree /repo/wt/gone\nHEAD 5555\ndetached\nprunable gitdir file points to non-existent location\n\n\
            worktree /repo/bare\nbare\n";
        let found: Vec<(String, Option<String>)> = parse_worktrees(porcelain)
            .into_iter()
            .map(|w| (w.path, w.branch))
            .collect();
        let want =
            |path: &str, branch: Option<&str>| (path.to_string(), branch.map(str::to_string));
        assert_eq!(
            found,
            [
                want("/repo/main", Some("main")),
                want("/repo/wt/with space", Some("feature/login-form")),
                want("/repo/wt/detached", None),
                want("/repo/wt/locked", Some("locked-one")),
                want("/repo/wt/gone", None),
                want("/repo/bare", None),
            ]
        );
        assert!(parse_worktrees("").is_empty());
    }

    #[test]
    fn git_s_repository_location_variables_do_not_move_the_current_worktree() {
        let sandbox = crate::testing::Sandbox::empty();
        let here = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        crate::testing::init_repo(here.path(), "main");
        crate::testing::init_repo(other.path(), "main");
        // git reports the resolved path, and macOS puts tempdirs behind /private.
        let here = std::fs::canonicalize(here.path()).unwrap();
        let other = std::fs::canonicalize(other.path()).unwrap();
        let other_git = other.join(".git");

        for name in REPOSITORY_LOCATION_ENV {
            let value = if name == "GIT_WORK_TREE" {
                &other
            } else {
                &other_git
            };
            let _var = crate::testing::EnvVar::set(&sandbox, name, value);
            assert_eq!(
                current_worktree(Some(&here)).as_deref(),
                Some(here.to_string_lossy().as_ref()),
                "{name}"
            );
        }
    }

    #[test]
    fn a_worktree_list_git_cannot_give_is_an_error_not_an_empty_list() {
        // Empty would read as "no worker is running here", which is what lets a dispatch
        // past the limit.
        let dir = tempfile::tempdir().unwrap();
        assert!(linked_worktrees(&dir.path().to_string_lossy()).is_err());
    }

    #[test]
    fn nwo_takes_the_last_two_segments_of_every_url_shape() {
        for url in [
            "git@github.com:acme/widget.git",
            "https://github.com/acme/widget",
            "https://github.com/acme/widget.git/",
            "ssh://git@github.com/acme/widget.git",
            "git://github.com/acme/widget",
        ] {
            assert_eq!(nwo_from_url(url).as_deref(), Some("acme/widget"), "{url}");
        }
    }

    #[test]
    fn nwo_is_none_when_there_is_no_remote() {
        assert_eq!(nwo_from_url(""), None);
        assert_eq!(nwo_from_url("widget"), None);
    }

    /// The repository's own hub, which is what every caller asking for no hub in
    /// particular gets.
    fn slugify(nwo: &str) -> String {
        slug_for(nwo, None)
    }

    #[test]
    fn slug_collapses_separators_and_drops_the_rest() {
        assert!(slugify("acme/widget").starts_with("acme-widget-"));
        assert!(slugify("Acme/Widget.Team").starts_with("acme-widget-team-"));
        assert!(slugify("acme/my_widget").starts_with("acme-my-widget-"));
        assert!(slugify("acme/ウィジェット").starts_with("acme-"));

        // The readable half is not the address. These four collapse to the same characters
        // and used to be one slug, which meant one inbox: a report filed against any of
        // them arrived at whichever was asked about first.
        let together = [
            "acme/foo-bar",
            "acme/foo_bar",
            "acme/foo.bar",
            "acme-foo/bar",
        ];
        let slugs: std::collections::BTreeSet<String> =
            together.iter().map(|n| slugify(n)).collect();
        assert_eq!(slugs.len(), together.len(), "{slugs:?}");
        // Non-ASCII names collapsed to a bare prefix and shared it with each other.
        assert_ne!(slugify("acme/ウィジェット"), slugify("acme/ガジェット"));

        // Pinned to the value FNV-1a defines rather than to whatever this build computes:
        // the point of not using `DefaultHasher` is that an upgrade must not silently
        // re-address every inbox on the machine, and only a fixed expectation catches that.
        assert_eq!(fnv1a("acme/widget"), 0x8984_4950_9108_182c);
        assert_eq!(slugify("acme/widget"), "acme-widget-898449509108182c");
        // The config looks a repository up case-insensitively, so the address it derives
        // has to agree: one registered repository, one inbox.
        assert_eq!(slugify("Acme/Widget"), slugify("acme/widget"));
    }

    #[test]
    fn two_owners_of_the_same_name_get_different_hubs() {
        assert_ne!(
            hub_name("orgA/app", None).unwrap(),
            hub_name("orgB/app", None).unwrap()
        );
        assert_eq!(
            hub_name("acme/widget", None).unwrap(),
            format!("{HUB_PREFIX}{}", slugify("acme/widget"))
        );
        assert!(
            hub_name("acme/widget", None)
                .unwrap()
                .starts_with("adjutant-acme-widget-")
        );
    }

    #[test]
    fn a_name_with_no_ascii_left_is_an_error_rather_than_a_bare_prefix() {
        assert!(hub_name("ウィジェット", None).is_err());
    }

    /// The compatibility lock. There are hub records, inboxes and archives on disk under
    /// the slug this used to produce, and a hub whose address moved on upgrade is a hub
    /// nothing can reach and a pile of reports nobody reads. Pinned to the literal rather
    /// than to `slugify` so that a change to *either* side fails here.
    #[test]
    fn asking_for_no_hub_in_particular_addresses_exactly_what_it_addressed_before() {
        assert_eq!(
            slug_for("acme/widget", None),
            "acme-widget-898449509108182c"
        );
        assert_eq!(slug_for("acme/widget", None), slugify("acme/widget"));
        assert_eq!(
            hub_name("acme/widget", None).unwrap(),
            "adjutant-acme-widget-898449509108182c"
        );
        // An identifier that is empty says nothing, so it addresses the same hub rather
        // than a nameless second one. Every gate in front of this drops blanks; this is
        // the one that has to hold when one of them is bypassed.
        assert_eq!(slug_for("acme/widget", Some("")), slugify("acme/widget"));
    }

    /// The point of the split: a second hub is a second address, not a second repository.
    #[test]
    fn a_hub_identifier_moves_the_address_and_nothing_else() {
        let plain = slug_for("acme/widget", None);
        let feature = slug_for("acme/widget", Some("wid-957"));
        assert_ne!(plain, feature);
        assert_ne!(feature, slug_for("acme/widget", Some("wid-958")));
        // Same repository, so the readable half still says which one — that is what a
        // person picks out of a state directory listing.
        assert!(feature.starts_with("acme-widget-wid-957-"), "{feature}");
        // Pinned like the plain slug beside it, and for the same reason: FNV-1a is written
        // out here precisely so that an address never moves under a running hub.
        assert_eq!(feature, "acme-widget-wid-957-5283c95d4f4cc314");
        // Case-folded like the repository half. `--hub WID-957` and `--hub wid-957` are
        // one hub, the way `Acme/Widget` and `acme/widget` are one repository.
        assert_eq!(slug_for("acme/widget", Some("WID-957")), feature);

        // The digest covers both halves. Identifiers that collapse to the same readable
        // characters would otherwise share an inbox — the same collapse the digest was
        // added to undo for repository names.
        let together = ["foo-bar", "foo_bar", "foo.bar"];
        let slugs: std::collections::BTreeSet<String> = together
            .iter()
            .map(|hub| slug_for("acme/widget", Some(hub)))
            .collect();
        assert_eq!(slugs.len(), together.len(), "{slugs:?}");
        // And it covers the repository half too, so one identifier does not merge two
        // repositories into one hub.
        assert_ne!(feature, slug_for("acme/gadget", Some("wid-957")));

        // An identifier with nothing readable in it still gets its own address, and still
        // reads as this repository rather than as a name ending in a stray separator.
        let opaque = slug_for("acme/widget", Some("ウィジェット"));
        assert_eq!(opaque, "acme-widget-75ea31bdae83ea93");
        assert_ne!(opaque, plain);
        assert_ne!(opaque, slug_for("acme/widget", Some("ガジェット")));

        assert_eq!(
            hub_name("acme/widget", Some("wid-957")).unwrap(),
            format!("{HUB_PREFIX}{feature}")
        );
    }

    #[test]
    fn worktree_fallback_matches_the_convention_a_neighbour_tool_would_use() {
        // Under the main checkout rather than beside it: the procedure's "am I in a
        // worktree" check looks for this path, and a scattered sibling would not match it.
        assert_eq!(
            worktree_fallback(DEFAULT_WORKTREE_PATTERN, "/src/widget", "someone/WID-957").unwrap(),
            "/src/widget/.claude/worktrees/someone-WID-957"
        );
    }

    #[test]
    fn the_branch_carries_the_owner_and_the_task_name() {
        assert_eq!(
            branch_fallback(DEFAULT_BRANCH_PATTERN, "someone", "app-1234"),
            "someone/app-1234"
        );
        // Two trackers, two keys, no collision — because the key is in the name.
        assert_ne!(
            branch_fallback(DEFAULT_BRANCH_PATTERN, "someone", "wid-233"),
            branch_fallback(DEFAULT_BRANCH_PATTERN, "someone", "xyz-233")
        );
    }

    #[test]
    fn worktree_fallback_honours_a_nested_pattern() {
        assert_eq!(
            worktree_fallback(".worktrees/{name}", "/src/widget", "feat/x").unwrap(),
            "/src/widget/.worktrees/feat-x"
        );
    }

    #[test]
    fn worktree_fallback_leaves_absolute_patterns_alone() {
        assert_eq!(
            worktree_fallback("/tmp/wt/{name}", "/src/widget", "feat/x").unwrap(),
            "/tmp/wt/feat-x"
        );
    }

    #[test]
    fn a_branch_cannot_be_used_to_walk_out_of_the_checkout() {
        // `{name}` flattens slashes and so can only name one directory; `{branch}` keeps
        // them on purpose, which is also what let `..` through.
        for branch in ["../../../tmp/x", "feat/../../x", "..", "/etc/passwd", "  "] {
            assert!(
                worktree_fallback(DEFAULT_WORKTREE_PATTERN, "/src/widget", branch).is_err(),
                "{branch} was accepted"
            );
        }
        // A dot inside a component is a normal branch name and stays one.
        assert_eq!(
            worktree_fallback("{branch}", "/src/widget", "release/1.2.x").unwrap(),
            "/src/widget/release/1.2.x"
        );
    }
}
