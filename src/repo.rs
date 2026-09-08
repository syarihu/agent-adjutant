//! Where we are: the main checkout, the repo's canonical `owner/name`, and the hub name
//! derived from it.
//!
//! The hub name IS the address a worker sends its report to, so the derivation lives here
//! and nowhere else. A rule spelled out in the hub's prompt and again in the worker's is a
//! rule with two versions, and the day they disagree the report goes silently nowhere.

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
    pub slug: String,
    pub hub_name: String,
    /// `origin`, `argument`, or `dirname` — how `nwo` was arrived at. The commands warn on
    /// `dirname` because that is the one case where two machines can disagree.
    pub nwo_source: &'static str,
}

fn git(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output, String> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output().map_err(|e| format!("cannot run git: {e}"))
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
pub fn slugify(nwo: &str) -> String {
    let readable: String = nwo
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '.' | '_' | '/' => '-',
            c => c,
        })
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .collect();
    // Nothing readable left is still an error at `hub_name`, where it can say so. A slug
    // that is only a digest names nothing a person could recognise.
    if readable.is_empty() {
        return String::new();
    }
    // Of the *lowercased* name, so that the address is as case-insensitive as the config
    // lookup beside it. Hashing the original made `Acme/Widget` and `acme/widget` two
    // addresses for one registered repository — a hub started from one and a worker from
    // the other, each writing to an inbox the other never reads.
    format!("{readable}-{:016x}", fnv1a(&nwo.to_lowercase()))
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

pub fn hub_name(nwo: &str) -> Result<String, String> {
    let slug = slugify(nwo);
    if slug.is_empty() {
        return Err(format!(
            "cannot build a hub name from the repository name ({nwo})"
        ));
    }
    Ok(format!("{HUB_PREFIX}{slug}"))
}

pub fn resolve(repo_arg: Option<&str>) -> Result<RepoInfo, String> {
    resolve_in(None, repo_arg)
}

/// Same, but answering for `start` rather than for the process's own directory. An MCP
/// server is started once per session and then asked about whichever checkout the caller is
/// sitting in, which is not necessarily the one it was launched from.
pub fn resolve_in(start: Option<&Path>, repo_arg: Option<&str>) -> Result<RepoInfo, String> {
    let main = main_worktree(start)?;
    let (nwo, source) = match repo_arg {
        Some(arg) => (arg.to_string(), "argument"),
        None => name_with_owner(&main),
    };
    let hub_name = hub_name(&nwo)?;
    Ok(RepoInfo {
        repo: nwo.rsplit('/').next().unwrap_or(&nwo).to_string(),
        slug: slugify(&nwo),
        hub_name,
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_ne!(hub_name("orgA/app").unwrap(), hub_name("orgB/app").unwrap());
        assert_eq!(
            hub_name("acme/widget").unwrap(),
            format!("{HUB_PREFIX}{}", slugify("acme/widget"))
        );
        assert!(
            hub_name("acme/widget")
                .unwrap()
                .starts_with("adjutant-acme-widget-")
        );
    }

    #[test]
    fn a_name_with_no_ascii_left_is_an_error_rather_than_a_bare_prefix() {
        assert!(hub_name("ウィジェット").is_err());
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
