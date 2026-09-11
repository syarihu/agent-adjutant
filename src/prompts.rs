//! The three procedures, baked into the binary.
//!
//! They are shipped as MCP prompts rather than written into each agent's commands directory
//! because a file that is copied into place is a file that drifts: upgrade the tool and the
//! copies stay behind, one per config directory, all subtly different. A prompt served over
//! MCP is a pointer — one upgrade moves every caller at once.
//!
//! The same text is also reachable as a tool (`adjutant_skill`), because MCP prompt support
//! is uneven across agents and a procedure nobody can fetch is a procedure nobody follows.

pub struct PromptDef {
    pub name: &'static str,
    pub raw_content: &'static str,
}

pub const PROMPTS: [PromptDef; 3] = [
    PromptDef {
        name: "adj-hub",
        raw_content: include_str!("../commands/adj-hub.md"),
    },
    PromptDef {
        name: "adj-worker",
        raw_content: include_str!("../commands/adj-worker.md"),
    },
    PromptDef {
        name: "adj-report",
        raw_content: include_str!("../commands/adj-report.md"),
    },
];

/// The one thing worth spending always-on context on.
///
/// The 1500 lines of procedure matter only while a hub or a worker is running, and both
/// fetch them on purpose. This is the exception: a worker cannot ask for the report
/// procedure unless it already knows that reporting is a thing it is allowed to do.
pub const INSTRUCTIONS: &str = "\
adjutant hands work out to workers and takes their bug reports back in.
While you are working a task, a bug you find OUTSIDE that task is not yours to fix and not
yours to file: an unrelated fix pollutes this task's diff, and a diff nobody can review is a
diff nobody can revert. Hand it over instead — `adj skill adj-report` (or adjutant_skill,
name=adj-report), follow it, go back to your task. Hub: `adj hub`. Worker: adj-worker.";

pub fn find(name: &str) -> Option<&'static PromptDef> {
    PROMPTS.iter().find(|p| p.name == name)
}

/// Split a leading `---` block off the top, returning its lines and the body.
///
/// The frontmatter is an agent-specific header (a description, a tool allowlist) that means
/// nothing over MCP, but the description in it is the one-line summary a prompt listing
/// wants — so it is parsed rather than merely dropped.
pub fn strip_frontmatter(raw: &str) -> (Vec<(String, String)>, &str) {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return (vec![], raw);
    };
    let Some(end) = rest.find("\n---\n") else {
        return (vec![], raw);
    };
    let (header, body) = rest.split_at(end);
    let fields = header
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.trim().to_string()))
        })
        .collect();
    (fields, body["\n---\n".len()..].trim_start_matches('\n'))
}

pub fn description(prompt: &PromptDef) -> String {
    let (fields, _) = strip_frontmatter(prompt.raw_content);
    fields
        .iter()
        .find(|(k, _)| k == "description")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| prompt.name.to_string())
}

/// The body with `$ARGUMENTS` filled in.
///
/// A procedure invoked with nothing to say is normal (the hub is just starting up), so the
/// placeholder is replaced with an empty string rather than left as literal text for the
/// reader to puzzle over.
pub fn render(prompt: &PromptDef, arguments: &str) -> String {
    let (_, body) = strip_frontmatter(prompt.raw_content);
    body.replace("$ARGUMENTS", arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prompt_has_a_description_for_the_listing() {
        for prompt in &PROMPTS {
            let description = description(prompt);
            assert!(!description.is_empty(), "{}", prompt.name);
            assert_ne!(
                description, prompt.name,
                "{} has no frontmatter description",
                prompt.name
            );
        }
    }

    #[test]
    fn the_frontmatter_never_reaches_the_reader() {
        for prompt in &PROMPTS {
            let body = render(prompt, "");
            assert!(!body.starts_with("---"), "{}", prompt.name);
            assert!(!body.contains("allowed-tools:"), "{}", prompt.name);
        }
    }

    #[test]
    fn arguments_land_where_the_procedure_expects_them() {
        let report = find("adj-report").unwrap();
        assert!(
            report.raw_content.contains("$ARGUMENTS"),
            "adj-report has nowhere to put what the worker found"
        );
        let rendered = render(report, "検索結果の画像が縦に潰れる");
        assert!(rendered.contains("検索結果の画像が縦に潰れる"));
        assert!(!rendered.contains("$ARGUMENTS"));
    }

    #[test]
    fn a_body_with_no_frontmatter_is_left_alone() {
        let (fields, body) = strip_frontmatter("# heading\n\ntext\n");
        assert!(fields.is_empty());
        assert_eq!(body, "# heading\n\ntext\n");
    }

    #[test]
    fn an_unterminated_frontmatter_is_not_swallowed() {
        let (fields, body) = strip_frontmatter("---\ndescription: x\n# heading\n");
        assert!(fields.is_empty());
        assert!(body.starts_with("---"));
    }

    /// The one line the brief carries about where the branch came from has to be read back.
    ///
    /// The hub decides the base with its `baseBranch` rule and writes the answer into the
    /// brief; nothing else records it. §5 had no step that read it, so every PR fell through
    /// to a bare `gh pr create` — which targets the repository's default branch. A task
    /// branched off a release branch then opened a PR carrying every commit the release
    /// branch has and the default branch does not, and merging it put the release into the
    /// default branch.
    #[test]
    fn the_pr_step_takes_its_base_from_the_brief() {
        let worker = section(find("adj-worker").unwrap().raw_content, "## 5. ");
        // Matched against the section with its whitespace squeezed out. The procedures are
        // hard-wrapped, so a phrase to look for is as likely as not to be split across two
        // lines — and re-wrapping a paragraph must not decide whether this guard holds.
        let flowed: String = worker.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            worker.contains("--base"),
            "the PR step never passes a base: {worker}"
        );
        // The label as the brief spells it, not the bare noun: the step after this one says
        // 「step 2 のベースブランチを渡す」, so a check for the word alone was satisfied by that
        // sentence — and renaming the label the step goes looking for passed the guard while
        // leaving the worker hunting for a line the hub does not write.
        assert!(
            flowed.contains("指示書の「ベースブランチ」行"),
            "the PR step does not say where the base comes from: {worker}"
        );
        // The hub writes a commit-ish (`origin/release/1.2`), which is not a branch name
        // GitHub will accept — so the step has to say to strip the remote. Asserting on
        // `origin/` alone was satisfied by the examples that illustrate the stripping, so
        // the instruction itself could go while the guard stayed green.
        assert!(
            flowed.contains("`origin/`を外した"),
            "the PR step does not say to strip the remote from the brief's value: {worker}"
        );

        // The base is checked before anyone is asked to review, not after. A bot asked to
        // read a PR opened against the wrong base reviews a diff of everything the base was
        // missing, and correcting the base afterwards does not re-run that review — so §6
        // ends up triaging it.
        let checks_base = flowed
            .find("baseRefName")
            .expect("the PR step never checks the base it landed on");
        let asks_for_review = flowed
            .find("request_copilot_review")
            .expect("the PR step never asks for a review");
        assert!(
            checks_base < asks_for_review,
            "the base is checked after the review is requested"
        );

        // Both sides spell the label the same way. Renaming it in the hub's brief template
        // without telling the worker is what left the line unread in the first place.
        let hub = find("adj-hub").unwrap().raw_content;
        assert!(
            hub.contains("- ベースブランチ: {base_branch}"),
            "the brief template no longer writes the line the worker is told to read"
        );
    }

    /// A `gh pr create` with nothing to base it on is the defect itself, not just one
    /// wording of it: the flag is easy to drop when the surrounding prose is rewritten.
    ///
    /// The flag has to be inside the command, which is why this reads the command's own span
    /// instead of the characters around it. A first version looked within 40 characters of
    /// the occurrence and passed a stripped `gh pr create` — the sentence that follows it
    /// explains what omitting `--base` does, so the word was there either way, and the guard
    /// was reading the explanation rather than the command.
    #[test]
    fn no_procedure_opens_a_pull_request_without_saying_what_to_base_it_on() {
        let mut found = 0usize;
        for prompt in &PROMPTS {
            for (at, _) in prompt.raw_content.match_indices("gh pr create") {
                found += 1;
                let tail = &prompt.raw_content[at..];
                // The command runs to the end of its backticks, or of its line in a fenced
                // block. Anything past that is prose about the command.
                let end = tail.find(['`', '\n']).unwrap_or(tail.len());
                let command = &tail[..end];
                assert!(
                    command.contains("--base"),
                    "{} opens a PR with no base: {command}",
                    prompt.name
                );
            }
        }
        assert!(
            found > 0,
            "no procedure opens a pull request at all any more"
        );
    }

    /// One numbered section of a procedure, from its heading to the next one.
    ///
    /// Section-scoped rather than whole-file: `--base` anywhere in 400 lines would satisfy
    /// a substring check while the step that opens the PR carried none.
    fn section(raw: &str, heading: &str) -> String {
        // Anchored to the start of a line: `## 5. ` also matches inside `### 5. `, which is a
        // heading the procedures really use, and a guard that quietly read the wrong section
        // would pass by finding nothing to object to.
        let anchored = format!("\n{heading}");
        let at = raw
            .find(&anchored)
            .unwrap_or_else(|| panic!("no section starting `{heading}`"));
        let rest = &raw[at + anchored.len()..];
        let end = rest.find("\n## ").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn the_always_on_instructions_carry_the_rule_and_its_reason() {
        // A worker that knows the rule and not the reason weighs it against whatever it was
        // just asked to do, and loses — observed, with the bug fixed in place instead.
        let text = INSTRUCTIONS.to_lowercase();
        assert!(text.contains("not yours to fix"), "{INSTRUCTIONS}");
        assert!(
            text.contains("diff"),
            "the reason is missing: {INSTRUCTIONS}"
        );
        assert!(text.contains("adj skill adj-report"), "{INSTRUCTIONS}");
        assert!(text.contains("adj hub"), "{INSTRUCTIONS}");
    }

    #[test]
    fn the_always_on_instructions_stay_short_enough_to_always_be_on() {
        assert!(
            INSTRUCTIONS.lines().count() <= 5,
            "instructions are {} lines",
            INSTRUCTIONS.lines().count()
        );
    }

    #[test]
    fn the_procedures_name_no_repository_but_this_one() {
        // The prompts ship inside the binary, so anything left over from where they grew up
        // would be published with it.
        for prompt in &PROMPTS {
            let text = prompt.raw_content.to_lowercase();
            for stray in ["dotfiles", "taskhub", "syarihu"] {
                assert!(
                    !text.contains(stray),
                    "{} still mentions {stray}",
                    prompt.name
                );
            }
        }
    }

    /// Placeholder identifiers the procedures are allowed to use as examples.
    ///
    /// The checks below are shaped as "does this match the placeholder vocabulary" rather
    /// than "is this one of the names known to be private". A denylist only catches what
    /// someone already thought of — and it would have to spell the private names out in a
    /// file that gets published, which is the problem it exists to solve. A whitelist of
    /// made-up values catches the leak nobody predicted.
    ///
    /// **A value only belongs here once it has been checked against the real config it is
    /// standing in for.** The first version of this list was written from what the
    /// procedures already contained, and three of the entries turned out to be live tracker
    /// keys — so the guard was certifying exactly the values it existed to catch. A
    /// whitelist assembled from the thing it is auditing audits nothing.
    const PLACEHOLDER_KEYS: [&str; 7] = ["ALPHA", "BETA", "GAMMA", "WID", "ABC", "XYZ", "WEB"];

    /// Words that wear a tracker key's shape without being one.
    ///
    /// `UTF-8` is a run of capitals, a hyphen and a digit, which is exactly what `ABC-819`
    /// is; nothing about the text can tell them apart. So the rule keeps its shape and the
    /// exceptions are named here — and an entry earns its place the same way a placeholder
    /// does, by being checked against the real config it is *not* standing in for. Adding a
    /// word here because a test went red is how the guard stops guarding.
    /// `UTF-8` is an encoding, `FNV-1a` a hash, and `KEY-123` is this file describing the
    /// shape it looks for.
    const NOT_TRACKER_KEYS: [&str; 3] = ["UTF", "FNV", "KEY"];

    fn is_a_key(candidate: &str) -> bool {
        !NOT_TRACKER_KEYS.contains(&candidate)
    }

    #[test]
    fn the_key_scan_reads_both_shapes_a_key_is_written_in() {
        assert_eq!(
            issue_keys_in(r#"担当は ALPHA-233 なのだ"#),
            vec!["ALPHA".to_string()]
        );
        // Bare, in value position — the shape a key is written in inside a config example,
        // and the one that used to go uninspected.
        assert_eq!(
            issue_keys_in(r#""issueKeys": { "example/team-app": "ALPHA" }"#),
            vec!["ALPHA".to_string()]
        );
        assert_eq!(
            issue_keys_in(r#""project": "ABC", "cloudId": "example.atlassian.net""#),
            vec!["ABC".to_string()]
        );
        // Prose capitals are not keys: only the right hand side of a colon is inspected.
        assert!(issue_keys_in("MCP と CLI の話なのだ").is_empty());
        // The value on the next line is ordinary JSON formatting, and used to slip past.
        assert_eq!(
            issue_keys_in("\"project\":\n  \"ABC\""),
            vec!["ABC".to_string()]
        );
    }

    #[test]
    fn every_issue_key_in_the_procedures_is_a_made_up_one() {
        // The guard has to be looking at something. A scan that finds nothing passes for
        // the same reason a scan that finds only placeholders does, and the audit this
        // came from was about exactly that kind of quiet agreement.
        let all: Vec<String> = PROMPTS
            .iter()
            .flat_map(|p| issue_keys_in(p.raw_content))
            .collect();
        assert!(all.len() >= 4, "the key scan found almost nothing: {all:?}");

        for prompt in &PROMPTS {
            for key in issue_keys_in(prompt.raw_content) {
                assert!(
                    PLACEHOLDER_KEYS.contains(&key.as_str()),
                    "{} uses the tracker key {key}, which is not one of {PLACEHOLDER_KEYS:?} \
                     — if it names a real project it must not ship",
                    prompt.name
                );
            }
        }
    }

    /// The same rule, applied to everything that ships rather than to the procedures alone.
    ///
    /// The guard above was written when the leak found was in the procedures, and it was
    /// scoped to them — so the *source* kept carrying live tracker keys (`APP`, `SEASONAL`,
    /// and issue numbers under them) in fixtures and doc comments for as long as the
    /// procedures were clean, and the clean procedures were read as the whole answer. What
    /// ships is the repository, so the repository is what gets read.
    #[test]
    fn nothing_that_ships_names_a_real_tracker_key() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut checked = 0usize;
        for path in shipped_files(root) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            checked += 1;
            for key in issue_keys_in(&text).into_iter().filter(|k| is_a_key(k)) {
                assert!(
                    PLACEHOLDER_KEYS.contains(&key.as_str()),
                    "{} uses the tracker key {key}, which is not one of {PLACEHOLDER_KEYS:?} \
                     — if it names a real project it must not ship",
                    path.display()
                );
            }
            for host in tracker_hosts_in(&text) {
                assert!(
                    host.starts_with("example."),
                    "{} names the tracker site {host} — a real site must not ship",
                    path.display()
                );
            }
        }
        // A sweep that walked nothing would pass for the same reason a clean one does.
        assert!(checked > 10, "only {checked} files were read");
    }

    /// Every text file the repository publishes: sources, tests, procedures, the example
    /// config, the READMEs. Not `target`, not `.git`.
    fn shipped_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Ok(read) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if path.is_dir() {
                out.extend(shipped_files(&path));
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs" | "md" | "json" | "toml" | "sh" | "yml" | "yaml")
            ) {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn a_host_named_right_after_japanese_text_is_found_rather_than_a_panic() {
        // The procedures are Japanese. Taking a byte index from `rfind` and adding one
        // landed inside a multi-byte character and panicked — so the guard crashed on
        // exactly the input it exists to inspect.
        assert_eq!(
            tracker_hosts_in("課題はexample.atlassian.net にあるのだ"),
            vec!["example.atlassian.net"]
        );
        // Assembled rather than written out. A literal here would be a hostname shipping in
        // the repository, and whether it belongs to anybody cannot be checked — every name
        // under `atlassian.net` resolves, so DNS answers yes to made-up ones too. The sweep
        // over the whole tree would flag it, and it would be right to.
        let elsewhere = format!("somewhere-else{}", ".atlassian.net");
        assert_eq!(
            tracker_hosts_in(&format!("サイトは`{elsewhere}`なのだ")),
            vec![elsewhere.clone()]
        );
        assert!(!elsewhere.starts_with("example."), "{elsewhere}");
        assert!(tracker_hosts_in("日本語だけなのだ").is_empty());
        // The suffix on its own is a description of the shape, not a site.
        assert!(tracker_hosts_in("the marker is `.atlassian.` here").is_empty());
    }

    #[test]
    fn every_tracker_host_in_the_procedures_is_a_made_up_one() {
        for prompt in &PROMPTS {
            for host in tracker_hosts_in(prompt.raw_content) {
                assert!(
                    host.starts_with("example."),
                    "{} names the tracker site {host} — a real site must not ship",
                    prompt.name
                );
            }
        }
    }

    /// Every tracker key in the text: the `KEY-123` shapes, and the bare keys.
    ///
    /// A key does not have to be attached to a number to be a real project. `"project":
    /// "WID"` and an `issueKeys` map are where a key is written on its own, and those are
    /// exactly the places a real one gets pasted in from a working config — but looking
    /// only for `KEY-123` meant a bare key was never inspected at all.
    fn issue_keys_in(text: &str) -> Vec<String> {
        let mut found = numbered_issue_keys_in(text);
        found.extend(bare_issue_keys_in(text));
        found.sort();
        found.dedup();
        found
    }

    /// Uppercase tokens written as a JSON string *value* — `: "ALPHA"`. Value position is
    /// what makes this narrow enough to be useful: prose is full of capitals, but the right
    /// hand side of a colon inside a config example is where a project key lives.
    fn bare_issue_keys_in(text: &str) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut found = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b':' {
                i += 1;
                continue;
            }
            i += 1;
            // Every JSON whitespace, not just the two that fit on one line: `"project":`
            // with its value on the next line is ordinary formatting, and stopping at the
            // newline made the guard skip exactly the case a real config is written in.
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'"' {
                continue;
            }
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' && bytes[i] != b'\n' {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'"' {
                continue;
            }
            let value = &text[start..i];
            i += 1;
            // `ABC-819` in value position is the same key as `ABC` — take the key half, so
            // one entry in the placeholder list covers both spellings.
            let key = value.split('-').next().unwrap_or(value);
            if key.len() >= 2 && key.chars().all(|c| c.is_ascii_uppercase()) {
                found.push(key.to_string());
            }
        }
        found
    }

    /// Every `KEY-123` shape in the text, as its key half.
    fn numbered_issue_keys_in(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut found = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].is_ascii_uppercase() {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && chars[i].is_ascii_uppercase() {
                i += 1;
            }
            let key: String = chars[start..i].iter().collect();
            // A key is two or more capitals followed by `-` and a digit. Shorter runs are
            // ordinary capitals, and a `-` with no digit after it is a hyphenated word.
            let is_issue_id = key.len() >= 2
                && chars.get(i) == Some(&'-')
                && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit());
            if is_issue_id {
                found.push(key);
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// Every hostname in the text that belongs to a hosted issue tracker.
    ///
    /// Walks bytes, not chars. A hostname is ASCII, so scanning back over ASCII host bytes
    /// and stopping at the first byte that is not one lands on a char boundary by
    /// construction — where taking `rfind`'s index and adding one does not, and panicked on
    /// the first Japanese character before a hostname. The procedures are written in
    /// Japanese, so that is the case this guard is *for*: it would have crashed instead of
    /// reporting the leak it exists to catch.
    fn tracker_hosts_in(text: &str) -> Vec<String> {
        fn is_host_byte(b: u8) -> bool {
            b.is_ascii_alphanumeric() || b == b'.' || b == b'-'
        }
        let lower = text.to_lowercase();
        let bytes = lower.as_bytes();
        let mut found = Vec::new();
        for marker in [".atlassian.", ".linear.", ".jira."] {
            let mut from = 0;
            while let Some(offset) = lower[from..].find(marker) {
                let at = from + offset;
                let mut start = at;
                while start > 0 && is_host_byte(bytes[start - 1]) {
                    start -= 1;
                }
                let mut end = at + marker.len();
                while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
                    end += 1;
                }
                // A hostname has something in front of the marker and something after it.
                // `".atlassian."` written on its own is a suffix being *described* — this
                // file describes it a few lines up — and reporting that as a site names
                // nobody while making the guard cry wolf about itself.
                if start < at && end > at + marker.len() {
                    found.push(lower[start..end].to_string());
                }
                from = at + marker.len();
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// A prose sweep once lowercased `... on Issue` in a GraphQL query, and the procedures
    /// ship inside the binary — so the first anyone would have known is a hub reporting a
    /// validation error from a query it was told to run.
    #[test]
    fn a_type_name_inside_a_fenced_block_keeps_its_case() {
        for prompt in &PROMPTS {
            let mut inside = false;
            for line in prompt.raw_content.lines() {
                if line.trim_start().starts_with("```") {
                    inside = !inside;
                    continue;
                }
                if !inside {
                    continue;
                }
                // GraphQL type conditions and type names are capitalised by definition.
                for word in line.split("... on ").skip(1) {
                    let name = word.split_whitespace().next().unwrap_or("");
                    assert!(
                        name.starts_with(|c: char| c.is_ascii_uppercase()),
                        "{}: `... on {name}` is not a type name",
                        prompt.name
                    );
                }
            }
        }
    }
}
