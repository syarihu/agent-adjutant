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
    for name in ["adj-hub", "adj-worker", "adj-report"] {
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
    assert!(opened >= 3, "found only {opened} gate examples");
}
