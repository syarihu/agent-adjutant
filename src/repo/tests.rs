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
    let want = |path: &str, branch: Option<&str>| (path.to_string(), branch.map(str::to_string));
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
fn the_host_of_a_remote_is_read_out_of_any_of_its_shapes() {
    for (url, host) in [
        ("git@github.com:acme/widget.git", "github.com"),
        ("https://github.com/acme/widget", "github.com"),
        (
            "https://user@Git.Example.com:8443/acme/widget.git",
            "git.example.com",
        ),
        ("ssh://git@github.com:22/acme/widget.git", "github.com"),
        ("git://git.example.com/acme/widget", "git.example.com"),
    ] {
        assert_eq!(host_from_url(url).as_deref(), Some(host), "{url}");
    }
    assert_eq!(host_from_url(""), None);
    assert_eq!(host_from_url("widget"), None);
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
    let slugs: std::collections::BTreeSet<String> = together.iter().map(|n| slugify(n)).collect();
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
