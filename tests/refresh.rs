//! `adj task refresh` and `adjutant_refresh`: records brought up to date with their pull
//! requests, with `gh` stubbed so nothing here reaches GitHub.

mod common;

use common::*;

const PULL: &str = "https://github.com/acme/widget/pull";

/// A `gh` that knows four pull requests, and writes down every one it is asked about.
///
/// 1 is merged, 2 is open, 3 was closed without merging, 5 is merged but its record is
/// removed while `gh` is answering, and anything else fails the way `gh` does for a PR it
/// cannot find.
fn stub_gh(fixture: &Fixture) -> (String, PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let asked = fixture.repo.join("gh-asked");
    let gh = stubs.join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\n\
             echo \"$3\" >> {asked}\n\
             case \"$3\" in\n\
             {PULL}/1) echo MERGED ;;\n\
             {PULL}/2) echo OPEN ;;\n\
             {PULL}/3) echo CLOSED ;;\n\
             {PULL}/5) grep -l '/pull/5\"' \"$ADJUTANT_STATE_DIR\"/tasks/*/*.json | xargs rm; echo MERGED ;;\n\
             *) echo 'GraphQL: Could not resolve to a PullRequest' >&2; exit 1 ;;\n\
             esac\n",
            asked = shell_quoted(&asked.to_string_lossy()),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Prepended rather than replacing: the binary still has to find the real `git`.
    let path = format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );
    (path, asked)
}

/// A record in `status`, pointing at `pr`.
fn task_with_pr(fixture: &Fixture, title: &str, status: &str, pr: &str) -> String {
    let added = fixture.json(&["task", "add", "--title", title, "--body", title, "--json"]);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--status",
        status,
        "--pr",
        pr,
        "--no-hand-over",
    ]);
    id
}

fn status_of(fixture: &Fixture, id: &str) -> String {
    let shown = fixture.json(&["task", "show", "--id", id]);
    shown["status"].as_str().unwrap().to_string()
}

fn ids(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn only_a_merged_pr_moves_its_task_to_done_and_the_rest_are_reported() {
    let fixture = Fixture::new(QUIET);
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let open = task_with_pr(&fixture, "open", "pr", &format!("{PULL}/2"));
    let closed = task_with_pr(&fixture, "closed", "pr", &format!("{PULL}/3"));
    let missing = task_with_pr(&fixture, "missing", "pr", &format!("{PULL}/4"));
    // Still being worked on, but its PR is already in: merged is merged.
    let working = task_with_pr(&fixture, "working", "dispatched", &format!("{PULL}/1"));
    let (path, _) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    let mut done = ids(&result["done"]);
    done.sort();
    let mut expected = vec![merged.clone(), working.clone()];
    expected.sort();
    assert_eq!(done, expected, "{result}");
    assert_eq!(ids(&result["open"]), [open.as_str()], "{result}");
    assert_eq!(ids(&result["closed"]), [closed.as_str()], "{result}");
    assert_eq!(ids(&result["unreadable"]), [missing.as_str()], "{result}");
    assert!(
        result["unreadable"][0]["error"]
            .as_str()
            .unwrap()
            .contains("Could not resolve"),
        "{result}"
    );

    assert_eq!(status_of(&fixture, &merged), "done");
    assert_eq!(status_of(&fixture, &working), "done");
    for id in [&open, &closed, &missing] {
        assert_eq!(status_of(&fixture, id), "pr", "{id} was moved");
    }
}

/// More records than are asked about at once: each answer still lands on its own record.
#[test]
fn every_record_gets_its_own_answer_past_the_first_batch() {
    let fixture = Fixture::new(QUIET);
    let prs = [1, 2, 3, 4];
    let made: Vec<(String, i32)> = (0..11)
        .map(|n| {
            let pr = prs[n % prs.len()];
            let id = task_with_pr(&fixture, &format!("t{n}"), "pr", &format!("{PULL}/{pr}"));
            (id, pr)
        })
        .collect();
    let (path, _) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for (id, pr) in &made {
        let bucket = match pr {
            1 => "done",
            2 => "open",
            3 => "closed",
            _ => "unreadable",
        };
        assert!(
            ids(&result[bucket]).contains(id),
            "{id} is not in {bucket}: {result}"
        );
        let want = if *pr == 1 { "done" } else { "pr" };
        assert_eq!(status_of(&fixture, id), want, "{id}");
    }
}

/// One record that cannot be moved does not take the others with it: the ones that were
/// moved are still reported, and the one that failed says why.
#[test]
fn a_record_that_cannot_be_moved_is_reported_and_the_rest_still_move() {
    let fixture = Fixture::new(QUIET);
    let gone = task_with_pr(&fixture, "gone", "pr", &format!("{PULL}/5"));
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let (path, _) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ids(&result["done"]), [merged.as_str()], "{result}");
    assert_eq!(ids(&result["failed"]), [gone.as_str()], "{result}");
    assert!(
        result["failed"][0]["error"]
            .as_str()
            .unwrap()
            .contains("no such task"),
        "{result}"
    );
    assert!(ids(&result["skipped"]).is_empty(), "{result}");
    assert_eq!(status_of(&fixture, &merged), "done");
}

/// A finished record is not looked up at all: there is nothing its PR could change.
#[test]
fn a_done_or_cancelled_task_is_not_asked_about() {
    let fixture = Fixture::new(QUIET);
    task_with_pr(&fixture, "done", "done", &format!("{PULL}/2"));
    let cancelled = task_with_pr(&fixture, "cancelled", "cancelled", &format!("{PULL}/1"));
    let (path, asked) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("No task is waiting on a pull request"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!asked.exists(), "gh was asked about a finished task");
    assert_eq!(status_of(&fixture, &cancelled), "cancelled");
}

/// Without `gh` there is no answer, and no answer is not "merged".
#[test]
fn without_gh_every_task_is_left_alone_and_said_to_be_unreadable() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let empty = fixture.repo.join("no-bin");
    std::fs::create_dir_all(&empty).unwrap();
    // Only what the binary needs besides `gh`: `git`, found where the system keeps it.
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let git_dir = std::path::Path::new(String::from_utf8_lossy(&git.stdout).trim())
        .parent()
        .unwrap()
        .to_path_buf();
    if git_dir.join("gh").exists() {
        // `gh` lives beside `git` here, so a PATH without one would not have the other.
        return;
    }
    let out = fixture
        .command(["task", "refresh", "--json"])
        .env(
            "PATH",
            format!("{}:{}", empty.to_string_lossy(), git_dir.to_string_lossy()),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ids(&result["unreadable"]), [id.as_str()], "{result}");
    assert_eq!(status_of(&fixture, &id), "pr");
}

#[test]
fn the_tool_does_what_the_command_does() {
    let fixture = Fixture::new(QUIET);
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let open = task_with_pr(&fixture, "open", "pr", &format!("{PULL}/2"));
    let (path, _) = stub_gh(&fixture);

    let mut child = fixture
        .command(["mcp"])
        .env("PATH", &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_refresh", "arguments": {
                "cwd": fixture.repo.to_str().unwrap(),
            }}),
        )
    )
    .unwrap();
    let out = child.wait_with_output().unwrap();
    let reply: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).lines().next().unwrap()).unwrap();
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    let result: serde_json::Value = serde_json::from_str(text).expect(text);

    assert_eq!(ids(&result["done"]), [merged.as_str()], "{result}");
    assert_eq!(ids(&result["open"]), [open.as_str()], "{result}");
    assert_eq!(status_of(&fixture, &merged), "done");
    assert_eq!(status_of(&fixture, &open), "pr");
}
