//! The procedures: that what they name exists, and how they are rendered for each agent.

mod common;

use common::*;

#[test]
fn every_command_and_tool_the_procedures_name_actually_exists() {
    // The procedures are prose telling an agent which commands to run. Rename a subcommand
    // or a tool and nothing here stops compiling — the agent just gets an error at the one
    // moment it was supposed to be getting work done.
    // Sanitised like every other child, though a help dump reads neither config nor
    // environment: an exemption is a thing the next reader has to re-derive.
    let help = String::from_utf8(
        Command::new(BIN)
            .arg("--help")
            .hermetic()
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let subcommands: Vec<String> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();
    assert!(
        subcommands.len() > 10,
        "could not read the subcommand list: {help}"
    );

    let fixture = Fixture::new(QUIET);
    let listed = mcp(&fixture, &[request(1, "tools/list", serde_json::json!({}))]);
    let tools: Vec<String> = listed[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();

    let mut missing: Vec<String> = Vec::new();
    for name in [
        "adj-hub",
        "adj-worker",
        "adj-report",
        "adj-review-hub",
        "adj-reviewer",
    ] {
        let text =
            std::fs::read_to_string(format!("{}/commands/{name}.md", env!("CARGO_MANIFEST_DIR")))
                .unwrap();
        // Only backticked mentions: `adjutant work` is a reference, "adjutant sets" is prose.
        // Both names are checked, because the procedures use them interchangeably — and a
        // check that only knew the long one stopped covering the launcher the day it was
        // renamed, without failing.
        for quoted in text.split('`').skip(1).step_by(2) {
            let Some(rest) = quoted
                .strip_prefix("adjutant")
                .or_else(|| quoted.strip_prefix("adj"))
            else {
                continue;
            };
            let referenced = match rest.chars().next() {
                Some('_') => rest.trim_start_matches('_').to_string(),
                Some(' ') => rest.split_whitespace().next().unwrap_or("").to_string(),
                // `adjutant` alone is the tool's own name, not a reference to anything.
                _ => continue,
            };
            let known = match rest.starts_with('_') {
                true => tools.contains(&format!("adjutant_{referenced}")),
                false => subcommands.contains(&referenced),
            };
            if !known && !referenced.is_empty() {
                missing.push(format!("{name}.md: adjutant {referenced}"));
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "the procedures name things that do not exist: {missing:#?}"
    );
}

#[test]
fn skill_formats_for_specified_agent() {
    let fixture = Fixture::new(QUIET);
    let claude = fixture.ok(&["skill", "adj-hub", "--agent", "claude"]);
    let agy = fixture.ok(&["skill", "adj-hub", "--agent", "agy"]);

    assert!(claude.contains("AskUserQuestion"));
    assert!(!agy.contains("AskUserQuestion"));
    assert!(agy.contains("ask_question"));
}

#[test]
fn skill_rejects_unknown_agent() {
    let fixture = Fixture::new(QUIET);
    let out = fixture
        .command(["skill", "adj-hub", "--agent", "invalid-agent"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("invalid value 'invalid-agent'"));
}

#[test]
fn the_worker_asks_adj_which_engine_reviews() {
    let text = std::fs::read_to_string(format!(
        "{}/commands/adj-worker.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();

    let start = text.find("\n### Choosing the review engine").unwrap();
    let end = text.find("\n#### Running a codex review").unwrap();
    let section = &text[start..end];

    assert!(section.contains("adj review-engine --json"));
    assert!(section.contains("Running a codex review"));
    assert!(section.contains("Running a Claude review"));
    assert!(section.contains("message"));

    assert!(!section.contains("rate-limit-cache.json"));
    assert!(!section.contains("which codex"));
    assert!(!section.contains("five_hour"));
    assert!(!section.contains("seven_day"));
}

/// The worker decides between waiting on a person and leaving a record by rules written in
/// its procedure. Those rules name a flag and a field the binary has to accept, and the
/// examples are what a worker copies, so both are checked against the real command.
#[test]
fn the_worker_procedure_says_when_diff_and_verify_wait_and_its_examples_open() {
    let text = std::fs::read_to_string(format!(
        "{}/commands/adj-worker.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    for needle in [
        "\"wait\": false",
        "`stoppedBy`",
        "`round-limit`",
        "`verify-failed`",
        "`manual-check`",
        "`unsure`",
        "`stop-at`",
        "Stop at",
        "`problem`",
        "`goal`",
        "`steps`",
        "`sections`",
        "`quick`",
        "`task`",
        "\"task\"",
    ] {
        assert!(text.contains(needle), "adj-worker.md does not say {needle}");
    }

    // Every `adj gate open` example, opened as written. serde drops a key it does not know,
    // so a misspelt field would open without complaint and vanish; each key has to come back.
    let fixture = Fixture::new(QUIET);
    let mut opened = 0;
    let mut rest = text.as_str();
    while let Some(start) = rest.find("adj gate open --json <<'JSON'\n") {
        let body = &rest[start..];
        let body = &body[body.find('\n').unwrap() + 1..];
        let end = body
            .find("\nJSON\n")
            .expect("an example without its JSON terminator");
        let mut example: serde_json::Value = serde_json::from_str(&body[..end])
            .unwrap_or_else(|e| panic!("an example is not JSON: {e}\n{}", &body[..end]));
        rest = &body[end..];

        example["worktree"] = serde_json::json!(fixture.repo.to_str().unwrap());
        let file = fixture.repo.join("gate.json");
        std::fs::write(&file, example.to_string()).unwrap();
        let out = fixture.json(&["gate", "open", "--file", file.to_str().unwrap(), "--json"]);
        for key in example.as_object().unwrap().keys() {
            assert!(
                out["gate"].get(key).is_some(),
                "the example's {key} did not survive being opened: {out}"
            );
        }
        opened += 1;
    }
    assert!(opened >= 5, "found only {opened} gate examples");
}

#[test]
fn every_flag_in_the_parent_hub_forward_command_exists() {
    let help = String::from_utf8(
        Command::new(BIN)
            .args(["task", "add", "--help"])
            .hermetic()
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();

    let text = std::fs::read_to_string(format!(
        "{}/commands/adj-hub.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();

    let heading = "\n### A key that is not one of your children";
    let start = text
        .find(heading)
        .unwrap_or_else(|| panic!("missing {heading} in commands/adj-hub.md"));
    let rest = &text[start + heading.len()..];
    let end = match (rest.find("\n### "), rest.find("\n## ")) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => rest.len(),
    };
    let step = &rest[..end];

    let code_start = step
        .find("```bash\n")
        .expect("missing bash code block in child check step");
    let code_body = &step[code_start + 8..];
    let code_end = code_body
        .find("\n```")
        .expect("unterminated code block in child check step");
    let code = &code_body[..code_end];

    let add_line = code
        .lines()
        .find(|l| l.contains("adj task add"))
        .expect("missing adj task add line in code block");

    let flags: Vec<&str> = add_line
        .split_whitespace()
        .filter(|token| token.starts_with("--"))
        .collect();
    assert!(!flags.is_empty(), "expected --flags in line: {add_line}");

    for flag in flags {
        assert!(
            help.lines()
                .any(|line| line.split_whitespace().next() == Some(flag)),
            "flag `{flag}` from forward command not found in `adj task add --help`:\n{help}"
        );
    }
}

#[test]
fn skill_section_cli_and_mcp() {
    let fixture = Fixture::new(QUIET);
    let core = fixture.ok(&["skill", "adj-worker"]);
    assert!(core.contains("## Index of triggers"));
    assert!(!core.contains("## 5. Open a PR"));

    let pr = fixture.ok(&["skill", "adj-worker", "--section", "pr"]);
    assert!(pr.contains("## 5. Open a PR"));
    assert!(!pr.contains("## 1. Plan"));

    let both = fixture.ok(&["skill", "adj-worker", "--section", "pr,done"]);
    assert!(both.contains("## 5. Open a PR"));
    assert!(both.contains("## 9. Done: ask for cleanup"));
    let pr_idx = both.find("## 5. Open a PR").unwrap();
    let done_idx = both.find("## 9. Done: ask for cleanup").unwrap();
    assert!(pr_idx < done_idx);

    let all = fixture.ok(&["skill", "adj-worker", "--section", "all"]);
    assert!(all.contains("## Index of triggers"));
    assert!(all.contains("## 5. Open a PR"));
    assert!(all.contains("## 9. Done: ask for cleanup"));

    let err = fixture
        .command(["skill", "adj-worker", "--section", "nope"])
        .output()
        .unwrap();
    assert!(!err.status.success());
    let stderr = String::from_utf8_lossy(&err.stderr);
    assert!(stderr.contains("no such section of adj-worker: nope"));
    assert!(stderr.contains("core / all / plan / tools / implement"));

    let replies = mcp(
        &fixture,
        &[
            request(
                1,
                "tools/call",
                serde_json::json!({
                    "name": "adjutant_skill",
                    "arguments": {
                        "name": "adj-worker",
                        "section": "pr",
                    }
                }),
            ),
            request(
                2,
                "prompts/get",
                serde_json::json!({
                    "name": "adj-worker",
                    "arguments": {
                        "section": "pr",
                    }
                }),
            ),
        ],
    );
    assert_eq!(replies.len(), 2, "{replies:#?}");

    let tool_call_res: serde_json::Value =
        serde_json::from_str(replies[0]["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let tool_text = tool_call_res["content"].as_str().unwrap();
    assert!(tool_text.contains("## 5. Open a PR"));
    assert!(!tool_text.contains("## 1. Plan"));
    let sections = tool_call_res["sections"].as_array().unwrap();
    assert!(sections.contains(&serde_json::json!("pr")));

    let prompt_get_res = &replies[1]["result"];
    let prompt_text = prompt_get_res["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(prompt_text.contains("## 5. Open a PR"));
    assert!(!prompt_text.contains("## 1. Plan"));
}
