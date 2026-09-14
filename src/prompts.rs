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

    /// A dispatch can be told what to branch from, and saying so must not touch the config.
    ///
    /// `baseBranch` is a repository-level key, so answering 「`feature/x` から生やして」 by
    /// rewriting it sends every later task in that repository to the feature branch too —
    /// silently, because the next dispatch reads the same key and cannot tell it was set for
    /// one task.
    #[test]
    fn the_worktree_step_takes_a_base_meant_for_one_dispatch() {
        let create = step(find("adj-hub").unwrap().raw_content, "### 3. ");
        // Matched against the step with its whitespace squeezed out, for the reason the
        // base-branch guard above gives: the procedures are hard-wrapped, so re-wrapping a
        // paragraph must not decide whether this holds.
        let flowed: String = create.chars().filter(|c| !c.is_whitespace()).collect();
        // Which of the two wins. Without this the step can describe a per-dispatch base and
        // still leave the reader to guess whether the config overrides it.
        assert!(
            flowed.contains("指定された分岐元がconfigの`baseBranch`に優先する"),
            "the worktree step does not say the per-dispatch base wins: {create}"
        );
        assert!(
            flowed.contains("`baseBranch`を書き換えるのは誤り"),
            "the worktree step does not say to leave the repository-wide default alone: {create}"
        );
        // The value has to be turned into the remote-tracking form. `git worktree add` takes
        // a commit-ish, and a bare `feature/x` resolves to nothing in a worktree with no
        // local branch of that name — and the brief would then carry a spelling the base
        // line is not otherwise written in.
        assert!(
            flowed.contains("`origin/`付きのcommit-ishに揃える"),
            "the worktree step does not normalise the base it was handed: {create}"
        );
        // A base that does not resolve has to stop the dispatch. Falling through to the
        // default puts the worktree — and the pull request that follows it — on a branch
        // nobody asked for, and the brief then records that answer as though it were chosen.
        assert!(
            flowed.contains("gitrev-parse--verify"),
            "the worktree step never checks that the base it was handed exists: {create}"
        );
        // …against refs that were refreshed. A fetch that does not prune leaves the
        // remote-tracking ref of a branch deleted upstream in place, so the check above
        // answers "it exists" for a base that is gone and the failure surfaces later, at
        // `gh pr create`. Written as a ban on the bare form because the step fetches twice —
        // once for a base it was handed, once for the newest release branch — and both reads
        // are wrong in the same way.
        assert!(
            !create.contains("git fetch origin"),
            "the worktree step fetches without pruning, so a deleted base still resolves: {create}"
        );
        assert!(
            flowed.contains("gitfetch--pruneorigin"),
            "the worktree step does not refresh the remote before resolving a base: {create}"
        );
        assert!(
            flowed.contains("既定に落とさずユーザーに聞く"),
            "the worktree step falls back to the default base in silence: {create}"
        );
        // Only the base moves. A per-dispatch base that also renamed the branch would break
        // the lookup 「既存の worktree に手を入れたいと言われたら」 does by key.
        assert!(
            flowed.contains("変わるのは分岐元だけ"),
            "the worktree step does not say the branch and worktree names stay put: {create}"
        );
    }

    /// The brief's parent-task line is written by one side and read by the other.
    ///
    /// Same failure mode as the base-branch line: the hub can fill in a field nothing looks
    /// at and nothing goes red — the worker simply never learns what its task hangs off, and
    /// re-derives from one subtask the design the siblings already settled.
    #[test]
    fn the_parent_task_line_is_written_by_the_hub_and_read_by_the_worker() {
        let hub = find("adj-hub").unwrap().raw_content;
        assert!(
            hub.contains("- 親タスク: {parent_task}"),
            "the brief template has no slot for the parent task"
        );
        let plan = section(find("adj-worker").unwrap().raw_content, "## 1. ");
        let flowed: String = plan.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("指示書の「親タスク」"),
            "the plan step never reads the parent task the brief carries: {plan}"
        );
        // Naming the field is not reading it. A step that mentions 親タスク and tells nobody
        // to open it leaves both guards green while the worker learns nothing it did not
        // already have.
        assert!(
            flowed.contains("そちらの本文とコメントも読む"),
            "the plan step names the parent task without saying to read it: {plan}"
        );
        // And reading it starts from its own URL. The tools above take the tracker and the
        // repository from the task's URL, so a parent on another repository of the same
        // board — or another host — is fetched from the child's, which answers with whatever
        // task happens to carry that number.
        assert!(
            flowed.contains("親タスクのURLから割り出す"),
            "the plan step reuses the task's own tracker for the parent: {plan}"
        );
    }

    /// A report carries two tasks: the one it was found in, and that one's parent.
    ///
    /// The report used to carry only the task the reporter was holding, and the hub decides
    /// placement — independent issue or sub-issue — against what the report names. A worker
    /// holding one subtask of a split-up piece of work therefore handed the hub that subtask,
    /// and a sibling bug got hung underneath it instead of beside it, where the real parent
    /// could never see it.
    #[test]
    fn a_report_carries_the_task_it_was_found_in_and_that_task_s_parent() {
        let report = find("adj-report").unwrap().raw_content;
        let compose = between(report, "### 2. ", "### 3. ");
        // Matched against the step with its whitespace squeezed out, for the reason the
        // base-branch guard above gives: the procedures are hard-wrapped, so re-wrapping a
        // paragraph must not decide whether this holds.
        let flowed: String = compose.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compose.contains("## 発見元"),
            "the report body has no slot for the task the bug was found in: {compose}"
        );
        assert!(
            compose.contains("## 親タスク"),
            "the report body has no slot for that task's parent: {compose}"
        );
        // Which line fills which slot. The two headings alone are satisfied by a template
        // that never says where either value comes from, and the reporter then guesses —
        // which is the original defect with one more heading on it.
        assert!(
            flowed.contains("発見元は`.claude/task-brief.md`の「作業対象」行から"),
            "the report does not say the 発見元 is the reporter's own task: {compose}"
        );
        // The reason the 発見元 is not re-pointed at the parent. Losing it is how the slot
        // gets "fixed" back into the single field it was split out of.
        assert!(
            flowed.contains("**指示書の「親タスク」行ではない**"),
            "the report no longer says why the 発見元 stays on the reporter's own task: {compose}"
        );
        assert!(
            flowed.contains("親タスクは指示書の「親タスク」行をそのまま写す"),
            "the report does not forward the brief's parent-task line: {compose}"
        );
        // A missing parent has to have a spelling, or the hub cannot tell "no parent" from
        // "the reporter forgot" and questions every report that is not a subtask.
        assert!(
            flowed.contains("無ければ`-`"),
            "the report does not say what to write when there is no parent: {compose}"
        );

        // Both sides spell the labels the same way, in both directions: the hub writes the
        // brief line the report reads, and reads the report heading the report writes.
        // Renaming one end without the other is what left the base-branch line unread.
        let hub = find("adj-hub").unwrap().raw_content;
        assert!(
            hub.contains("- 親タスク: {parent_task}"),
            "the brief template no longer writes the line the report is told to forward"
        );
        // A report whose parent is `-` is complete, not short of a field. Without this the
        // hub asks back on every report that is not a subtask — and the reporter is told to
        // answer nothing but `[質問]`, so that round trip lands in the middle of its task.
        let intake: String = step(hub, "### Step 1 — 読む")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            intake.contains("`-`なのは欠落ではない"),
            "the hub reads a report with no parent as one that is missing a field"
        );
        // `-` being fine is not the heading being optional. The list of fields the hub
        // insists on is what makes a report without the heading incomplete; drop 親タスク
        // from it and a report that never mentions a parent passes intake as complete.
        assert!(
            intake.contains("/発見元（依頼元がいま持っているタスク）/親タスク。"),
            "the hub no longer requires the parent heading a complete report carries"
        );
        // And the missing heading has to be spelled out as a gap, because the only other
        // reading available downstream is `-`: placement then falls to the 発見元 and the
        // sibling bug sinks under one subtask again, which is the defect this PR fixes.
        assert!(
            intake.contains("見出しが丸ごと無いのは欠落"),
            "the hub has no rule for a report whose parent heading is missing entirely"
        );
        assert!(
            intake.contains("**`-`と同じには扱わない**"),
            "the hub may read a missing parent heading as `-`, which re-files the bug wrong"
        );
        // Two tasks to look up, so two trackers to resolve. Reusing the 発見元's repository
        // for the parent is the mistake the worker's plan step is already guarded against
        // (`親タスクのURLから割り出す` above): boards carry issues from several repositories,
        // and the wrong one answers with whatever task happens to hold that number.
        assert!(
            intake.contains("トラッカーとrepoはそれぞれのURLから割り出す"),
            "the hub checks both tasks against a single tracker"
        );
        // The reason travels with the rule. Without it the two lookups get folded back into
        // one repository the next time this paragraph is tightened, and nothing goes red:
        // the lookup succeeds, it just answers about a different task.
        assert!(
            intake.contains("エラーも出さずに"),
            "the hub does not say why the wrong tracker is dangerous"
        );
        // The lookups above are named after a URL, and the 発見元 does not always have one:
        // a report from a worktree with no brief carries a key off the branch, and a worker
        // started on a request with no issue carries the request itself where the URL goes.
        // Without this the hub asks back for something the reporter cannot produce, or
        // assembles a URL out of a number and sends the next worker somewhere that is not
        // there.
        assert!(
            intake.contains("発見元にURLが無いこともある"),
            "the hub takes every report as naming its 発見元 by URL"
        );

        let placement: String = step(hub, "### Step 3 — 起票")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            placement.contains("報告の「親タスク」"),
            "the hub does not read the parent the report now carries"
        );
        // Naming the field is not using it. The whole point is which task the sub-issue
        // decision is taken against, so the instruction has to say that much.
        assert!(
            placement.contains("sub-issueにするかは、報告の「親タスク」に対して決める"),
            "the hub names the report's parent without placing the issue against it"
        );
        // And a report with no parent still has to be placeable, the way it was before.
        assert!(
            placement.contains("親タスクが`-`のときだけ、発見元に"),
            "the hub has no rule for a report whose parent is `-`"
        );

        // The parent then keeps travelling: the brief of the issue the hub just filed
        // carries it too, or the next worker re-derives from one subtask the design its
        // siblings already settled — the same loss, one hop further down.
        let dispatch: String = step(hub, "### Step 4 — 着手させる")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            dispatch.contains("「親タスク」は報告の「親タスク」をそのまま"),
            "the new brief does not carry the parent the report reported"
        );
        assert!(
            dispatch.contains("`-`なら発見元のタスク"),
            "the new brief has nothing to fall back to when the report carries no parent"
        );

        // The worker's own summary of what it hands over names the same fields the hub
        // asks for, and there are four of them now. This PR gave 親タスク a second meaning
        // — the brief's parent line — so a summary still calling the reporter's own task
        // 親タスク sends the worker to the wrong line of its brief; and one that drops the
        // parent instead spends a `[質問]` round trip in the middle of the worker's task,
        // which is the cost the parent heading was made required to avoid.
        let handover: String = section(find("adj-worker").unwrap().raw_content, "## 7. ")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(
            handover.contains("症状・`file:line`・発見元・親タスクを揃えて渡す"),
            "the worker summarises the hub's required fields as some other set: {handover}"
        );
    }

    /// A hub named for a parent task has to go and read that task.
    ///
    /// The machinery for standing one up was already there — an identifier moves the session
    /// name, the inbox and the record, and `--tab` opens a tab to put it in — but nothing
    /// told the hub that landed there what its own name meant. It started, collected the
    /// repository-wide dashboard, and waited: a hub scoped to one parent that could not name
    /// a single thing under it.
    #[test]
    fn a_hub_named_for_a_parent_task_reads_that_task_at_startup() {
        let scoped = section(find("adj-hub").unwrap().raw_content, "## 親タスクの hub");
        // Matched against the section with its whitespace squeezed out, for the reason the
        // base-branch guard above gives: the procedures are hard-wrapped, so re-wrapping a
        // paragraph must not decide whether this holds.
        let flowed: String = scoped.chars().filter(|c| !c.is_whitespace()).collect();
        // What tells this hub apart from the repository's own, out of what startup already
        // collected. A section that describes the behaviour without saying who it applies to
        // is one every hub reads as being about itself.
        assert!(
            flowed.contains("`adjutant_config`の`hub`に識別子が入っているhub"),
            "the section never says which hub it is about: {scoped}"
        );
        // The identifier is the key, which is what makes the subject readable without
        // storing it anywhere: reverse the key and the tracker and the repository fall out.
        assert!(
            flowed.contains("識別子は親タスクのキーそのもの"),
            "the section does not say the identifier is the parent's key: {scoped}"
        );
        assert!(
            flowed.contains("`issueKeys`で"),
            "the section does not reverse the key back to the issue's repository: {scoped}"
        );
        // Three levels, and the third is what says whether a subtask is actually done.
        assert!(
            flowed.contains("親タスク→その下のサブタスク→それらのサブタスクが開いたPR"),
            "the section does not say how far down to read: {scoped}"
        );
        // The collection is as heavy as the dashboard's and the hub lives as long, so it
        // goes the same way: to a sub-agent, without waiting for it.
        assert!(
            flowed.contains("サブエージェントに出す") && flowed.contains("結果を待たない"),
            "the startup step collects in the hub itself: {scoped}"
        );
        // And it asks rather than deciding. 「次はどれ」 taken from the tracker's own order
        // keeps the hub out of a dependency model it has no way to be right about.
        assert!(
            flowed.contains("トラッカーが返した順のまま"),
            "the startup step invents an order for the candidates: {scoped}"
        );
        assert!(
            flowed.contains("順序を発明しない"),
            "the startup step does not say to leave the ordering to the person: {scoped}"
        );
        // Asking must not mean stopping. 「起動時に AskUserQuestion を開かない」 is the
        // repository hub's rule and it holds here for the same reason — a hub waiting on an
        // answer is a hub not reading its inbox.
        assert!(
            flowed.contains("`AskUserQuestion`は開かない"),
            "the startup step stops the hub on a question: {scoped}"
        );
        // Re-read every time. That is what lets several of these run side by side and what
        // makes standing one up again tomorrow land where it left off.
        assert!(
            flowed.contains("状態を持たない"),
            "the section does not say to keep no state: {scoped}"
        );
        assert!(
            flowed.contains("起動のたびにこれを読み直し"),
            "the section keeps state without saying to re-read it: {scoped}"
        );
        // An identifier no source accounts for is not a licence to guess: a hub reporting on
        // the wrong parent is wrong silently.
        assert!(
            flowed.contains("どのソースにも当たらない識別子"),
            "the section has no answer for an identifier nothing accounts for: {scoped}"
        );
    }

    /// Reading a parent's children is the one step that differs per tracker.
    ///
    /// Everything else in this section is tracker-independent, so a single recipe written
    /// for whichever tracker was in front of the author would have been invisible until a
    /// repository on another one stood a hub up — and then it fails as an empty list, which
    /// reads exactly like a parent with nothing under it.
    #[test]
    fn a_parent_s_children_are_fetched_by_the_tracker_that_holds_them() {
        let fetch = step(find("adj-hub").unwrap().raw_content, "### 親の下を引く");
        let flowed: String = fetch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("/sub_issues"),
            "no recipe for GitHub sub-issues: {fetch}"
        );
        // The board holds statuses, not the parent-child relation, so the same sub-issue
        // call serves both GitHub types and only the status lookup is board-specific.
        assert!(
            flowed.contains("親子関係はissue側の属性"),
            "the board is treated as the owner of the parent-child relation: {fetch}"
        );
        assert!(
            flowed.contains("`parent={親キー}`"),
            "no recipe for Jira subtasks: {fetch}"
        );
        // The concrete form of 「順序を発明しない」: a sorted list read as a running order.
        assert!(
            flowed.contains("`ORDERBY`を付けない"),
            "the Jira query sorts the children, which reads as a running order: {fetch}"
        );
        assert!(
            flowed.contains("mcp__linear__list_issues"),
            "no recipe for Linear sub-issues: {fetch}"
        );
        // Pull requests are the exception: they land in the code repository whichever
        // tracker the tasks live in, so branching there would be four copies of one answer.
        assert!(
            flowed.contains("PRを探す先は、どのtypeでもコードリポジトリ1つ"),
            "the PR lookup branches by tracker as well: {fetch}"
        );
    }

    /// The brief's parent line is the payoff: a scoped hub always knows what to put in it.
    ///
    /// 「4. worker を起動する」 fills that line only when the dispatch was handed a parent,
    /// and tells the hub not to infer one from a sub-issue link. Left at that, a hub *named*
    /// for the parent still dispatched `-` — and its workers went on re-deriving from one
    /// subtask the design their siblings had already settled, which is the whole reason the
    /// line exists.
    #[test]
    fn a_hub_named_for_a_parent_task_puts_it_in_every_brief() {
        let dispatch = step(
            find("adj-hub").unwrap().raw_content,
            "### dispatch には自分の親タスクを載せる",
        );
        let flowed: String = dispatch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("指示書の「親タスク」行に自分の親タスクのURLを書く"),
            "the scoped hub does not fill the brief's parent line: {dispatch}"
        );
        assert!(
            flowed.contains("`-`にしない"),
            "the scoped hub is allowed to leave the parent line empty: {dispatch}"
        );
        // A report that names its own parent is the more specific answer, and Step 4 already
        // says which one to take. This must add a default, not overrule that.
        assert!(
            flowed.contains("報告が親タスクを名指ししているなら、そちらが優先"),
            "the scoped hub overwrites the parent a report reported: {dispatch}"
        );
        // Having a parent says nothing about where to branch from. Wiring the two together
        // would pick a base nobody asked for.
        assert!(
            flowed.contains("分岐元は動かさない"),
            "the scoped hub's parent moves the base branch too: {dispatch}"
        );
        // And the rule has to be reachable from the step that writes the brief: a reader of
        // 「4. worker を起動する」 who stops at 「無ければ `-`」 never learns about it.
        let spawn = step(find("adj-hub").unwrap().raw_content, "### 4. ");
        let spawn_flowed: String = spawn.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            spawn_flowed.contains("親タスクのhubはここが常に埋まる"),
            "the brief step's `-` default does not exempt a hub named for a parent: {spawn}"
        );
    }

    /// Offering a scoped hub is worth doing twice and worth not doing every time.
    ///
    /// The offer costs a tab and an `AskUserQuestion`, so its value is entirely in when it
    /// is *not* made: a hub that asks on every issue that happens to have children trains
    /// the person to dismiss it, and the two occasions where it helps get dismissed with
    /// the rest.
    #[test]
    fn a_scoped_hub_is_offered_for_a_parent_or_a_split_and_not_otherwise() {
        let offer = step(
            find("adj-hub").unwrap().raw_content,
            "### 親タスクの hub を提案する",
        );
        let flowed: String = offer.chars().filter(|c| !c.is_whitespace()).collect();
        // Who offers. A hub that is already inside the parent has nobody to offer it to.
        assert!(
            flowed.contains("提案するのはrepo自身のhubだけ"),
            "a hub already scoped to a parent offers another one: {offer}"
        );
        assert!(
            flowed.contains("条件は2つだけ"),
            "the offer has no closed set of conditions: {offer}"
        );
        assert!(
            flowed.contains("親タスクそのものを名指しされた"),
            "the offer does not cover being handed the parent itself: {offer}"
        );
        assert!(
            flowed.contains("細分化を依頼された"),
            "the offer does not cover being asked to split a task up: {offer}"
        );
        // The one that keeps the offer worth reading.
        assert!(
            flowed.contains("サブタスクを名指しされたときは提案しない"),
            "the offer fires on a subtask, where the person has already chosen: {offer}"
        );
        // Approval opens a tab and nothing else — the identifier spelled as the tracker
        // spells it. Not because the address moves with it (`slug_for` folds the identifier
        // before it digests it), but because that string is what the hub that lands there
        // shows, writes into a brief, and matches back against the config.
        assert!(
            flowed.contains("adjhub--tab--hub"),
            "the offer never says how to stand the hub up: {offer}"
        );
        assert!(
            flowed.contains("キーはトラッカーの綴りのまま渡す"),
            "the offer lets the identifier drift from the key: {offer}"
        );
        // Still the person's trigger. Opening a tab is not dispatching, on either side.
        assert!(
            flowed.contains("dispatchの引き金は人のまま"),
            "accepting the offer starts work by itself: {offer}"
        );
    }

    /// Nobody handed the hub the parent task, so the agent it sends has to go and get it.
    ///
    /// The section reversed the identifier back to a *source* and stopped there, while the
    /// brief the collector is given asked for the parent's title and URL — two things a key
    /// alone does not carry, and for Jira an URL the same file forbids assembling. Resolving
    /// it in the hub would have cost a lookup in the one block startup is allowed, so the
    /// agent that is already going to the tracker does it.
    #[test]
    fn the_agent_that_collects_a_parent_s_children_resolves_the_parent_itself() {
        let raw = find("adj-hub").unwrap().raw_content;
        let startup = step(raw, "### 起動時に読む");
        let startup_flowed: String = startup.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            startup_flowed.contains("hubが渡すのは識別子と、逆引きで判ったソースの情報だけ"),
            "the hub is left to resolve the parent before it can wait: {startup}"
        );
        assert!(
            startup_flowed.contains("収集エージェントが引く"),
            "nobody is told to read the parent task itself: {startup}"
        );
        // And the recipes have to exist, per tracker, or the collector is being asked for
        // something the procedure never shows it how to get.
        let fetch = step(raw, "### 親の下を引く");
        let fetch_flowed: String = fetch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            fetch_flowed.contains("親そのものを引く"),
            "the fetch step reads the children of a parent it never reads: {fetch}"
        );
        assert!(
            fetch_flowed.contains("--jq'{title,html_url,node_id}'"),
            "no URL for a GitHub parent: {fetch}"
        );
        // Jira's is the one that cannot be worked around by assembling a URL out of the key.
        assert!(
            fetch_flowed.contains("`webUrl`をそのまま使う"),
            "the Jira parent's URL is built rather than read: {fetch}"
        );
        // Linear needs a third thing: `list_issues` takes the parent's internal id, and the
        // identifier is a key.
        assert!(
            fetch_flowed.contains("mcp__linear__get_issue"),
            "no way to turn a Linear key into the parent: {fetch}"
        );
        assert!(
            fetch_flowed.contains("下を引くのに要る親のidもここで取る"),
            "the Linear parent's id is never obtained: {fetch}"
        );
        // The brief must not go back to asking for what the hub does not have.
        let brief = section(raw, "## Appendix — 親タスク収集エージェントへの指示書");
        let brief_flowed: String = brief.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            !brief_flowed.contains("{親タスクのタイトル}"),
            "the brief asks the hub for a title it cannot fill in: {brief}"
        );
        assert!(
            !brief_flowed.contains("{親タスクのURL}"),
            "the brief asks the hub for an URL it cannot fill in: {brief}"
        );
        assert!(
            brief_flowed.contains("親タスクのタイトルとURLは渡していないのだ"),
            "the brief never says who resolves the parent: {brief}"
        );
    }

    /// Every column of the sub-issue row is read by a step further down.
    ///
    /// The call started life as number, title and state — enough to print a list and nothing
    /// else. The URL the report's machine-readable rows require, the node id the board's
    /// status query is addressed by, and the repository that decides which `issueKeys` entry
    /// keys a child all come back from that same call, and none of them is recoverable
    /// afterwards without another round trip per child.
    #[test]
    fn a_sub_issue_row_carries_the_columns_its_readers_need() {
        let fetch = step(find("adj-hub").unwrap().raw_content, "### 親の下を引く");
        let flowed: String = fetch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("\\(.html_url)"),
            "the sub-issue rows carry no URL: {fetch}"
        );
        assert!(
            flowed.contains("\\(.node_id)"),
            "the sub-issue rows carry no node id: {fetch}"
        );
        assert!(
            flowed.contains("\\(.repository_url"),
            "the sub-issue rows carry no repository: {fetch}"
        );
        // The node id is dead weight unless the step that spends it says so.
        assert!(
            flowed.contains("`node_id`を渡す"),
            "nothing says what the node id is collected for: {fetch}"
        );
        // And spending it is not optional. The call was introduced as 「ボード上のステータスも
        // 要るなら」, while the collector's row demands a project item id and 「2. 着手を宣言する」
        // says not to fetch one twice — so a collector that reads this as optional hands back
        // `-` and the claim step pays for a GraphQL round trip it was told it would not need.
        assert!(
            flowed.contains("`nodes(ids:)`を1本打つ"),
            "the board call a `github-project` row depends on is optional: {fetch}"
        );
        assert!(
            flowed.contains("**projectitemid**がそこからしか出ず"),
            "nothing says the item id has no other source: {fetch}"
        );
        // A list cut off at the page boundary is indistinguishable from a short one, which
        // is the same reason this file gives for raising the board search's `--limit`.
        assert!(
            flowed.contains("ghapi--paginaterepos/"),
            "the sub-issue list stops at the first page: {fetch}"
        );
        assert!(
            flowed.contains("`nextPageToken`で辿る"),
            "the Jira subtask search stops at the first page: {fetch}"
        );
    }

    /// An identifier finds its source however it was typed, because the address already does.
    ///
    /// `slug_for` folds the identifier before it digests it, so `alpha-233` and `ALPHA-233`
    /// share one inbox and one record. Had the reverse lookup stayed case-sensitive, the
    /// lowercase spelling would have stood a hub up at the right address that could not name
    /// its own parent — and reports filed there would have gone unread by a hub sitting on
    /// them. Matching without regard to case is also what `adjutant_config` does to find the
    /// repository entry beside it.
    #[test]
    fn an_identifier_finds_its_source_whatever_case_it_was_typed_in() {
        let raw = find("adj-hub").unwrap().raw_content;
        let scoped = section(raw, "## 親タスクの hub");
        let flowed: String = scoped.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("綴りの大小は見ない"),
            "the reverse lookup leaves case undefined: {scoped}"
        );
        assert!(
            flowed.contains("大文字小文字を無視して突き合わせる"),
            "the reverse lookup does not say how it matches: {scoped}"
        );
        // Which makes two sources answering to one key reachable — the config only warns
        // about a duplicated key, and it compares the spellings exactly.
        assert!(
            flowed.contains("複数のソースに当たったら、どれかに決めない"),
            "the reverse lookup picks one of several matching sources: {scoped}"
        );
        // And the offer must not go on claiming the address moves with the spelling.
        let offer = step(raw, "### 親タスクの hub を提案する");
        let offer_flowed: String = offer.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            offer_flowed.contains("箱は動かない"),
            "the offer says a drifting spelling files the hub elsewhere: {offer}"
        );
    }

    /// Being asked to split a subtask up is being handed a parent.
    ///
    /// 「サブタスクを名指しされたときは提案しない」 and 「細分化を依頼された」 both fit that
    /// request, and read as written the exclusion wins — which would have declined to offer
    /// exactly where the offer pays for itself, since the split is about to give that issue
    /// children. The exclusion is about being told to *start* on a subtask.
    #[test]
    fn being_asked_to_split_a_subtask_up_still_earns_a_hub() {
        let offer = step(
            find("adj-hub").unwrap().raw_content,
            "### 親タスクの hub を提案する",
        );
        let flowed: String = offer.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("ただし「このサブタスクを割りたい」と言われたときは提案する"),
            "the exclusion swallows the request to split a subtask up: {offer}"
        );
        assert!(
            flowed.contains("除外が効くのは、そのサブタスクに**着手して**と言われたとき"),
            "the exclusion never says which request it is about: {offer}"
        );
    }

    /// The branch a subtask's PR is on is a convention, so it is resolved rather than spelled.
    ///
    /// Both the PR lookup and the worktree match were written against `{user}/{キー}`, which is
    /// only the default: a source carrying its own `branchPattern` produces something else, and
    /// `--head` is an exact match, so a subtask with a PR and a worktree was listed as having
    /// neither. Matching on 「ブランチ名にキーが入っているか」 instead traded that for the
    /// opposite error — `ALPHA-1` is a substring of `ALPHA-10`, so a subtask picks up its
    /// neighbour's worktree, which is worse: it reads as started work and the number beside it
    /// disappears. The convention has one owner, and `adj worktree-path` is who answers for it —
    /// the same call 「3. worktree を作る」 makes, so the expectation and the real branch come
    /// out of one place.
    #[test]
    fn a_subtask_s_branch_is_resolved_from_the_convention_rather_than_spelled_out() {
        let raw = find("adj-hub").unwrap().raw_content;
        let fetch = step(raw, "### 親の下を引く");
        let fetch_flowed: String = fetch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            fetch_flowed.contains("ブランチは子ごとに解決する"),
            "the branch is still assumed to have one shape for every child: {fetch}"
        );
        assert!(
            fetch_flowed.contains("adjworktree-path--name"),
            "nothing resolves a child's branch from the convention: {fetch}"
        );
        // Linear is the exception the source recipe already carved out: its branch is a field
        // on the issue, not a pattern, so running it through the convention invents a name the
        // tracker will never show anyone.
        assert!(
            fetch_flowed.contains("`linear`だけは打たない"),
            "a Linear child's branch is rebuilt instead of read: {fetch}"
        );
        assert!(
            fetch_flowed.contains("`gitBranchName`を落とさない"),
            "the Linear fetch drops the field its branch comes from: {fetch}"
        );
        // And the resolved value is what the lookup is given. Leaving the literal
        // `{user}/{サブタスクのキー}` in the command keeps the exact-match defect while the
        // paragraph above claims it is fixed.
        assert!(
            fetch_flowed.contains("--head'{解決したブランチ}'"),
            "the PR lookup still spells the branch out by hand: {fetch}"
        );
        // The remaining gap is now only the PR whose branch left the convention entirely. Kept
        // because the caveat is what stops 「PR が無い」 being read as 「未着手」.
        assert!(
            fetch_flowed.contains("`--search`が見るのはタイトルと本文で、ブランチ名ではない"),
            "the fallback search is left looking like it covers the gap: {fetch}"
        );

        // The collector matches worktrees against the same resolved value, and by equality.
        let brief = section(raw, "## Appendix — 親タスク収集エージェントへの指示書");
        let brief_flowed: String = brief.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            brief_flowed.contains("解決したブランチとの文字列一致"),
            "the collector matches worktrees by something other than the resolved branch: {brief}"
        );
        // Named as a ban, because containment is the reading a reader arrives at on their own
        // once the shapes stop being uniform — and it is wrong in the silent direction.
        assert!(
            brief_flowed.contains("キーが入っているかで探さないのだ"),
            "the collector may match a worktree by key containment: {brief}"
        );
        assert!(
            brief_flowed.contains("`ALPHA-1`が`ALPHA-10`"),
            "nothing says why containment matching is wrong: {brief}"
        );
        // `git worktree list --porcelain` prints `branch refs/heads/x`, so equality against a
        // short branch name matches nothing at all on the machine with no convention tool.
        assert!(
            brief_flowed.contains("`refs/heads/`が付いていたら外してから比べる"),
            "the equality match is defeated by the ref prefix git prints: {brief}"
        );
        // Equality has one failure mode of its own: a worktree created under a convention this
        // resolution does not reproduce now matches nothing, silently. Reporting the leftovers
        // is the only signal that proctor and `worktree-path` have drifted apart.
        assert!(
            brief_flowed.contains("どのサブタスクにも当たらなかったworktreeは"),
            "a worktree matching no child is dropped, hiding a drifted convention: {brief}"
        );
        // The row is the only channel to the hub, so the resolved branch has to travel on it —
        // as a column of the row, not merely as a word in the prose beside it.
        assert!(
            brief_flowed.contains("{status}|{assigneeまたは-}|{解決したブランチ}|{title}"),
            "the row the collector returns has no column for the resolved branch: {brief}"
        );
        assert!(
            brief_flowed.contains("**`{解決したブランチ}`も必ず入れるのだ**"),
            "the branch column is optional, so it is the one that gets dropped: {brief}"
        );
    }

    /// A source with no board says a task is started with a label, and that has to arrive.
    ///
    /// 「ボード上で着手済みのもの」 is the whole in-progress test the classification had, and a
    /// plain `github` source has no board for it to read — its own recipe calls a label and an
    /// open PR the signal. A started subtask carrying only the label therefore lands in 「次の
    /// 候補」, i.e. it is offered as unstarted work. The column has to exist before the
    /// classification can consult it: the sub-issue call is the only place the labels are
    /// reachable without one more round trip per child.
    #[test]
    fn a_started_subtask_is_recognised_where_the_source_marks_it_with_a_label() {
        let raw = find("adj-hub").unwrap().raw_content;
        let fetch = step(raw, "### 親の下を引く");
        let fetch_flowed: String = fetch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            fetch_flowed.contains("\\([.labels[].name]"),
            "the sub-issue rows carry no labels: {fetch}"
        );
        // A column nobody is told the purpose of is a column the next edit deletes.
        assert!(
            fetch_flowed.contains(
                "ラベルは、ボードを持たない素の`github`で着手済みを見分ける唯一の手掛かり"
            ),
            "nothing says what the labels are collected for: {fetch}"
        );
        let startup = step(raw, "### 起動時に読む");
        let startup_flowed: String = startup.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            startup_flowed
                .contains("ボードを持たない素の`github`では、進行中ラベルが付いているものも進行中"),
            "a labelled subtask on a boardless source is offered as a candidate: {startup}"
        );
        // And the collector has to put it somewhere the hub reads. The machine-readable row
        // is the only channel between them.
        let brief = section(raw, "## Appendix — 親タスク収集エージェントへの指示書");
        let brief_flowed: String = brief.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            brief_flowed.contains("ボードの無い素の`github`では`{status}`にラベルを入れるのだ"),
            "the label is fetched and then dropped before the hub sees it: {brief}"
        );
    }

    /// The candidate list the hub printed is the one the number is answered against.
    ///
    /// Handing the answer to 「2. タスクに着手」 alone lands in 「1. タスクを選ぶ」, which fetches
    /// every `taskSources` entry of the repository and keeps only what is assigned to the user
    /// or to nobody. A parent's children are collected under no such condition, so a subtask
    /// already assigned to someone else — or one living in a repository the board does not
    /// list — is printed as a candidate and then absent from the list the route rebuilds.
    /// That list also numbers itself, so the numbers stop meaning the same rows.
    #[test]
    fn a_number_answered_to_a_parent_hub_is_read_against_the_list_it_printed() {
        let startup = step(find("adj-hub").unwrap().raw_content, "### 起動時に読む");
        let flowed: String = startup.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("番号はこの一覧の行に当てる"),
            "the number is answered against some other list: {startup}"
        );
        assert!(
            flowed.contains("「1.タスクを選ぶ」で一覧を作り直さない"),
            "the route re-runs the repository-wide selection: {startup}"
        );
        // The reason has to travel with the rule, or a reader who finds the selection step
        // more thorough overrules it.
        assert!(
            flowed.contains("他人にアサイン済みのサブタスク"),
            "nothing says which candidates the rebuilt list drops: {startup}"
        );
        // And the key alone is not enough to carry: the claim step reads an item id it is
        // told not to fetch twice, which for this hub comes from the collector's rows.
        assert!(
            flowed.contains("「2.着手を宣言する」へ進み"),
            "the route does not say where the decided key goes instead: {startup}"
        );
        assert!(
            flowed.contains("収集の機械行がその行に持っているrepo・itemid"),
            "the claim step is left to re-fetch what the collection already reported: {startup}"
        );
    }

    /// Not rebuilding the list must not mean skipping what the list step does after fetching.
    ///
    /// 「1. タスクを選ぶ」 is a fetch *and* four rules and a question, and 「一覧を作り直さない」 sent
    /// the route past all of it. The three that matter here are the ones the parent collection
    /// cannot have already applied, because it deliberately asks a wider question than the
    /// repository route does: it ignores assignment, and it follows sub-issue links into
    /// repositories `issueKeys` never heard of. So a child assigned to someone else was offered
    /// as a candidate and claiming it overwrote their assignment on the two trackers that
    /// replace rather than append; a child from an unkeyed repository was offered and then
    /// stalled at the branch name; and the worker-or-worktree question the worktree step's last
    /// line demands an answer to was never asked.
    #[test]
    fn the_parent_route_keeps_the_selection_step_s_rules_when_it_skips_the_fetch() {
        let startup = step(find("adj-hub").unwrap().raw_content, "### 起動時に読む");
        let flowed: String = startup.chars().filter(|c| !c.is_whitespace()).collect();
        // What exactly is skipped. Without this the reader has one sentence saying "do not
        // rebuild the list" and no boundary on how much of that step it covers.
        assert!(
            flowed.contains("作り直さないのは一覧だけ"),
            "skipping the fetch is still written as skipping the whole step: {startup}"
        );

        // 1. Assignment. The classification has to carry the children the repository route
        // would have filtered out, and it must not number them as work to pick up.
        assert!(
            flowed.contains("**他人が持っている**"),
            "children assigned to someone else have nowhere to go but 次の候補: {startup}"
        );
        assert!(
            flowed.contains("**番号は振らない**"),
            "a child someone else holds is offered as a candidate to start: {startup}"
        );
        assert!(
            flowed.contains("着手する前に1回確認する"),
            "the route claims a child someone else holds without asking: {startup}"
        );
        // The cost of going ahead is not symmetric across trackers, and the confirmation is
        // worth nothing if it does not say so: `--add-assignee` adds, the other two replace.
        assert!(
            flowed.contains("**置き換え**"),
            "the confirmation does not say what claiming costs on Jira and Linear: {startup}"
        );
        assert!(
            flowed.contains("他人のアサインが本当に消える"),
            "nothing says the other person's assignment is lost, not shared: {startup}"
        );

        // 2. Repositories with no key. Same line the dashboard collection already reports, so
        // that a child from an unkeyed repository is accounted for rather than silently listed.
        assert!(
            flowed.contains("キー未設定のため対象外:"),
            "a child from an unkeyed repository is dropped or offered in silence: {startup}"
        );
        assert!(
            flowed.contains("番号を振ると「3.worktreeを作る」で詰まる"),
            "nothing says why an unkeyed child cannot be numbered: {startup}"
        );

        // 3. The route question. Its answer is read at the end of the worktree step, so a
        // dispatch that never asks arrives there with nothing to branch on.
        assert!(
            flowed.contains("「workerに任せる/worktreeだけ」を`AskUserQuestion`で聞く"),
            "the route never asks how far to take the task: {startup}"
        );
        // And it has to be reconciled with the rule two paragraphs above, or the two read as a
        // contradiction and the more emphatic one — the ban — wins.
        assert!(
            flowed.contains("開かないのは起動時の話"),
            "asking here reads as breaking the startup ban on questions: {startup}"
        );
        // The precedent that does skip the question is not comparable, and saying which route
        // it is stops it being copied here.
        assert!(
            flowed.contains("Step4がここを飛ばせるのはworker起動が確定している経路だから"),
            "the dispatch route's skip is left looking like a general licence: {startup}"
        );

        // The columns those rules read have to be collected, or the rules have nothing to run
        // on: assignment is not recoverable later without another round trip per child.
        let fetch = step(find("adj-hub").unwrap().raw_content, "### 親の下を引く");
        let fetch_flowed: String = fetch.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            fetch_flowed.contains("\\([.assignees[].login]"),
            "the sub-issue rows carry no assignee: {fetch}"
        );
        assert!(
            fetch_flowed.contains("\"assignee\"]"),
            "the Jira subtask search does not ask for the assignee: {fetch}"
        );
        // A column nobody is told the purpose of is a column the next edit deletes.
        assert!(
            fetch_flowed.contains("**`assignees`は「他人が持っている」を分けるため**"),
            "nothing says what the assignees are collected for: {fetch}"
        );
        // Comparing them needs the viewer's own identity, spelled the way each tracker spells
        // it — a display name matches nothing on GitHub and is not unique on Jira.
        assert!(
            fetch_flowed.contains("自分が誰かは、そのトラッカーの言い方で取る"),
            "the assignee comparison has no other side: {fetch}"
        );

        // And it has to reach the hub. The machine-readable row is the only channel.
        let brief = section(
            find("adj-hub").unwrap().raw_content,
            "## Appendix — 親タスク収集エージェントへの指示書",
        );
        let brief_flowed: String = brief.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            brief_flowed.contains("{assigneeまたは-}"),
            "the collected assignee is dropped before the hub sees it: {brief}"
        );
    }

    /// Counting a Linear parent's children starts from the key, on both routes that count.
    ///
    /// 「親の下を引く」 already says the hub holds a key and no internal id, and resolves one
    /// before listing children. The offer asks the same tracker the same question one step
    /// earlier, and was left handed 「親の id を渡して」 — an id that exists nowhere at that
    /// point. It fails as a count of zero, which reads exactly like a task with no children:
    /// the offer is then silently never made for any Linear parent.
    #[test]
    fn the_offer_resolves_a_linear_parent_from_its_key_before_counting() {
        let offer = step(
            find("adj-hub").unwrap().raw_content,
            "### 親タスクの hub を提案する",
        );
        let flowed: String = offer.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flowed.contains("`mcp__linear__get_issue`にキーを渡して親のidを取り"),
            "the offer counts Linear children off an id it was never given: {offer}"
        );
        assert!(
            flowed.contains("hubが持っているのはキーだけで、idは持っていない"),
            "the offer does not say why the key has to be resolved first: {offer}"
        );
    }

    /// The span between two headings, for a section `section` cannot hold.
    ///
    /// `adj-report` §2 is a fenced block whose lines are the report's own `## ` headings, so
    /// `section` — which ends at the next line starting `## ` — stops at the first line of
    /// the template and reads none of the prose that follows it. A guard scoped that way
    /// passes by finding nothing to object to.
    fn between(raw: &str, from: &str, to: &str) -> String {
        let start = format!("\n{from}");
        let at = raw
            .find(&start)
            .unwrap_or_else(|| panic!("no section starting `{from}`"));
        let rest = &raw[at + start.len()..];
        let end = rest
            .find(&format!("\n{to}"))
            .unwrap_or_else(|| panic!("no section starting `{to}` after `{from}`"));
        rest[..end].to_string()
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

    /// One `###` step of a procedure, from its heading to the next heading of any level.
    ///
    /// `section` stops at the next `## `, which for a `### ` step swallows every step after
    /// it — a phrase the guard looks for could then be satisfied by a neighbouring step
    /// while the one being inspected carried none.
    fn step(raw: &str, heading: &str) -> String {
        let body = section(raw, heading);
        let end = body.find("\n### ").unwrap_or(body.len());
        body[..end].to_string()
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
