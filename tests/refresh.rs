//! `adj task refresh` and `adjutant_refresh`: records brought up to date with their pull
//! requests, with `gh` stubbed so nothing here reaches GitHub.

mod common;

use common::*;

const PULL: &str = "https://github.com/acme/widget/pull";

/// One pull request as the refresh's GraphQL query answers it.
///
/// GitHub's upper-case values are passed in rather than written into the JSON: a bare
/// upper-case string after a colon reads as a tracker key to the guard over everything that
/// ships. `checks` are (status, conclusion) pairs of check runs, counted by state the way the
/// query asks for them; `review` is empty for no decision yet.
fn pr_json(state: &str, draft: bool, review: &str, checks: &[(&str, &str)]) -> String {
    let mut counts: std::collections::BTreeMap<&str, u32> = Default::default();
    for (status, conclusion) in checks {
        let state = if *status == "COMPLETED" {
            conclusion
        } else {
            status
        };
        *counts.entry(state).or_default() += 1;
    }
    let runs: Vec<String> = counts
        .iter()
        .map(|(state, count)| format!(r#"{{"state":"{state}","count":{count}}}"#))
        .collect();
    // Somebody is only waited on when GitHub says a review is required.
    let asked = u32::from(review == "REVIEW_REQUIRED");
    let review = if review.is_empty() {
        "null".to_string()
    } else {
        format!("\"{review}\"")
    };
    format!(
        r#"{{"state":"{state}","isDraft":{draft},"title":"A pull request","reviewDecision":{review},"reviewRequests":{{"totalCount":{asked}}},"latestOpinionatedReviews":{{"nodes":[]}},"commits":{{"nodes":[{{"commit":{{"statusCheckRollup":{{"contexts":{{"checkRunCountsByState":[{}],"statusContextCountsByState":[]}}}}}}}}]}}}}"#,
        runs.join(",")
    )
}

/// What `gh api graphql` prints for one pull request, `p0`.
fn answer_for(pr: &str) -> String {
    format!(r#"{{"data":{{"p0":{{"pullRequest":{pr}}}}}}}"#)
}

/// A `gh` that knows four pull requests, and writes down the number of every one it is asked
/// about, and every time it is run.
///
/// 1 is merged, 2 is open, 3 was closed without merging, 5 is merged but its record is
/// removed while `gh` is answering, and anything else fails the way `gh` does for a PR it
/// cannot find: an answer with the errors in it, and a non-zero exit.
fn stub_gh(fixture: &Fixture) -> (String, PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let asked = fixture.repo.join("gh-asked");
    let calls = fixture.repo.join("gh-calls");
    let gh = stubs.join("gh");
    // `-F pK=N` is how the query is given each pull request's number.
    let script = r#"#!/bin/sh
echo call >> @CALLS@
data=''
errors=''
for a in "$@"; do
  case "$a" in
    p[0-9]*=*)
      alias=${a%%=*}
      n=${a#*=}
      echo "$n" >> @ASKED@
      case "$n" in
        1) pr='@MERGED@' ;;
        2) pr='@OPEN@' ;;
        3) pr='@CLOSED@' ;;
        5) grep -l '/pull/5"' "$ADJUTANT_STATE_DIR"/tasks/*/*.json | xargs rm; pr='@MERGED@' ;;
        *) pr=null
           errors="$errors{\"type\":\"NOT_FOUND\",\"path\":[\"$alias\",\"pullRequest\"],\"message\":\"Could not resolve to a PullRequest with the number of $n.\"}," ;;
      esac
      data="$data\"$alias\":{\"pullRequest\":$pr},"
      ;;
  esac
done
data=${data%,}
errors=${errors%,}
if [ -n "$errors" ]; then
  echo "{\"data\":{$data},\"errors\":[$errors]}"
  echo 'gh: Could not resolve to a PullRequest' >&2
  exit 1
fi
echo "{\"data\":{$data}}"
"#
    .replace("@CALLS@", &shell_quoted(&calls.to_string_lossy()))
    .replace("@ASKED@", &shell_quoted(&asked.to_string_lossy()))
    .replace("@MERGED@", &pr_json("MERGED", false, "", &[]))
    .replace("@OPEN@", &pr_json("OPEN", false, "", &[]))
    .replace("@CLOSED@", &pr_json("CLOSED", false, "", &[]));
    std::fs::write(&gh, script).unwrap();
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
    // Whose turn each is, for the caller to say; a closed PR is the person's to decide.
    assert_eq!(result["closed"][0]["turn"], "closed", "{result}");
    assert_eq!(result["open"][0]["turn"], "unrequested", "{result}");
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

/// Many records in one query: each answer still lands on its own record.
#[test]
fn every_record_gets_its_own_answer_out_of_one_query() {
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
    // All of them in one query, however many there are.
    let calls = std::fs::read_to_string(fixture.repo.join("gh-calls")).unwrap();
    assert_eq!(calls.lines().count(), 1, "{calls}");
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
    // Only what the binary needs besides `gh`: `git`, linked into a directory of its own.
    // Not the directory `git` lives in, which is where `gh` is often installed too — on a CI
    // runner both are in /usr/bin — and a PATH through it would still find `gh`.
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let git = PathBuf::from(String::from_utf8_lossy(&git.stdout).trim());
    let only_git = fixture.repo.join("git-only-bin");
    std::fs::create_dir_all(&only_git).unwrap();
    std::os::unix::fs::symlink(&git, only_git.join("git")).unwrap();
    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &only_git)
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

/// A `gh` that answers with `answer` as the pull request it is asked about, read from a file so that an answer
/// longer than a pipe holds is no trouble to write.
fn stub_gh_answering(fixture: &Fixture, answer: &str) -> String {
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let said = fixture.repo.join("gh-answer");
    std::fs::write(&said, answer_for(answer)).unwrap();
    let gh = stubs.join("gh");
    std::fs::write(
        &gh,
        format!("#!/bin/sh\ncat {}\n", shell_quoted(&said.to_string_lossy())),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    )
}

fn record_file(fixture: &Fixture, id: &str) -> PathBuf {
    std::fs::read_dir(fixture.state.join("tasks"))
        .unwrap()
        .flatten()
        .map(|slug| slug.path().join(format!("{id}.json")))
        .find(|p| p.exists())
        .expect("the record's file")
}

/// What the card shows about a PR is read by the refresh and kept on the record, so the page
/// does not ask `gh` itself; and asking again with the same answer does not write the record.
#[test]
fn a_refresh_keeps_what_github_says_on_the_record_and_writes_only_when_it_changes() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "draft", "pr", &format!("{PULL}/9"));
    let path = stub_gh_answering(
        &fixture,
        &pr_json(
            "OPEN",
            true,
            "REVIEW_REQUIRED",
            &[("COMPLETED", "SUCCESS"), ("COMPLETED", "FAILURE")],
        ),
    );
    let refresh = || {
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
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };

    let result = refresh();
    assert_eq!(ids(&result["open"]), [id.as_str()], "{result}");
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(
        shown["prStatus"],
        serde_json::json!({
            "state": "draft",
            "title": "A pull request",
            "review": "required",
            "ci": { "pass": 1, "fail": 1, "pending": 0 },
        }),
        "{shown}"
    );
    assert_eq!(status_of(&fixture, &id), "pr");

    let file = record_file(&fixture, &id);
    let before = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    refresh();
    let after = std::fs::metadata(&file).unwrap().modified().unwrap();
    assert_eq!(
        before, after,
        "the record was rewritten with the same answer"
    );
}

/// A merged PR's summary is stored in the same write that moves the record to done.
#[test]
fn a_merged_pr_is_kept_as_merged_when_its_record_moves_to_done() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let (path, _) = stub_gh(&fixture);
    let out = fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["status"], "done");
    assert_eq!(shown["prStatus"]["state"], "merged", "{shown}");
}

/// An answer longer than a pipe holds. `gh` blocks writing it until
/// somebody reads, so a refresh that waited for it to exit first would time out and never
/// move the merged PR's record.
#[test]
fn a_pr_with_hundreds_of_checks_is_still_read() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "big", "pr", &format!("{PULL}/1"));
    let checks: Vec<(&str, &str)> = (0..2000)
        .map(|i| {
            if i % 100 == 0 {
                ("COMPLETED", "FAILURE")
            } else {
                ("COMPLETED", "SUCCESS")
            }
        })
        .collect();
    // Counted by state the way the query asks, the checks themselves are a few bytes; the
    // rest of the weight is a field the query never asked for, so the answer still fills a pipe.
    let mut answer = pr_json("MERGED", false, "APPROVED", &checks);
    answer.insert_str(
        answer.len() - 1,
        &format!(r#","padding":"{}""#, "x".repeat(200_000)),
    );
    assert!(answer.len() > 100_000, "{}", answer.len());
    let path = stub_gh_answering(&fixture, &answer);
    let out = fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["status"], "done", "{shown}");
    assert_eq!(shown["prStatus"]["ci"]["fail"], 20, "{shown}");
    assert_eq!(shown["prStatus"]["ci"]["pass"], 1980, "{shown}");
}

/// What was read about one PR does not describe the next one.
#[test]
fn pointing_a_record_at_another_pr_forgets_what_was_read_about_the_old_one() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "moved", "pr", &format!("{PULL}/9"));
    let path = stub_gh_answering(&fixture, &pr_json("OPEN", true, "", &[]));
    fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_eq!(
        fixture.json(&["task", "show", "--id", &id])["prStatus"]["state"],
        "draft"
    );
    // The same PR again keeps it; another one drops it.
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--pr",
        &format!("{PULL}/9"),
        "--no-hand-over",
    ]);
    assert!(fixture.json(&["task", "show", "--id", &id])["prStatus"].is_object());
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--pr",
        &format!("{PULL}/10"),
        "--no-hand-over",
    ]);
    assert!(fixture.json(&["task", "show", "--id", &id])["prStatus"].is_null());
}

/// A bare number is read on the host the repository's origin is on, not assumed to be
/// github.com's.
#[test]
fn a_bare_pr_number_is_read_on_the_host_of_the_origin() {
    let fixture = Fixture::new(QUIET);
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .hermetic()
            .args(args)
            .current_dir(&fixture.repo)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    };
    git(&[
        "remote",
        "set-url",
        "origin",
        "git@git.example.com:acme/widget.git",
    ]);
    let id = task_with_pr(&fixture, "bare", "pr", "2");
    let (path, _) = stub_gh(&fixture);
    // The stub's own log knows the numbers; what it was told about the host is in its args.
    let gh = fixture.repo.join("stub-bin").join("gh");
    let script = std::fs::read_to_string(&gh).unwrap().replace(
        "echo call >>",
        "echo \"$*\" | sed -n 's/.*--hostname \\([^ ]*\\).*/\\1/p' >> \"$ADJUTANT_STATE_DIR/gh-hosts\"\necho call >>",
    );
    std::fs::write(&gh, script).unwrap();
    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ids(&result["open"]), [id.as_str()], "{result}");
    let hosts = std::fs::read_to_string(fixture.state.join("gh-hosts")).unwrap();
    assert_eq!(hosts.trim(), "git.example.com");
}
