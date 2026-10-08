//! Keeping the cards' pull requests up to date without anybody asking (#227).
//!
//! The resident server asks GitHub which threads the person takes part in have changed, which
//! costs nothing when none has (a 304 does not count against the rate limit), and reads only
//! the PRs a card holds when one has. Placement then follows the PR's state, which `adj task
//! refresh` and the board's 「PR確認」 button read the same way, as a safety net.
//!
//! The poll never writes to GitHub. Marking a notification as read would take it out of the
//! person's own inbox, so nothing here sends a PATCH or a PUT.

use std::collections::{BTreeSet, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::branch_prs::BranchPrs;
use super::issue_parents::IssueParents;
use super::notifications::{self as gh, Notified};
use crate::registry::Context;
use crate::task::{self, PrRef, PrTurn, Task};

/// The only host polled. Notifications of another host would need its own `Last-Modified`
/// and its own login, and a refresh already reads those.
pub const HOST: &str = "github.com";

/// How long to wait when GitHub has not said, and when no card holds a PR on `HOST`: nothing
/// is asked then, and the records are looked at again after this.
const DEFAULT_INTERVAL: u64 = 60;

/// The longest the wait grows to while GitHub cannot be reached.
const BACKOFF_CAP: u64 = 900;

/// How often every card not yet merged is read anyway, along with any whose PR could not be read. GitHub does not notify the
/// person who did a thing, so a PR they merge, close or mark ready themselves sends nothing,
/// and a CI run that passes sends none either: without this, such a card would stay where it
/// is until the next refresh.
const STATE_REREAD: Duration = Duration::from_secs(5 * 60);

/// How long one round may spend waiting on `gh`.
pub(super) const ROUND_TIMEOUT: Duration = Duration::from_secs(30);

/// What the poll remembers between rounds. Nothing here is kept on a record: it is only what
/// lets the next round be cheap, and it starts empty when the server does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PollState {
    /// The `Last-Modified` of the last answer that was acted on, sent back as
    /// `If-Modified-Since` so that GitHub can say 304.
    pub last_modified: Option<String>,
    /// What GitHub last asked in `X-Poll-Interval`.
    pub interval_secs: Option<u64>,
    /// Rounds in a row that could not read GitHub.
    pub failures: u32,
    /// What `gh` said when the last round failed, for the board to show.
    pub error: Option<String>,
    /// When the cards with a state were last read without a notification.
    pub last_reread: Option<Instant>,
}

/// How a round ended, for `next_state`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    NotModified {
        interval: Option<u64>,
    },
    /// There was news and the PRs it concerns were read (or none was a card's).
    Changed {
        last_modified: Option<String>,
        interval: Option<u64>,
    },
    /// GitHub answered but the PRs could not be read. `last_modified` stays where it was, so
    /// the same news comes again next round.
    Unread {
        interval: Option<u64>,
        why: String,
    },
    /// GitHub could not be asked.
    Failed(String),
}

pub(super) fn next_state(prev: &PollState, outcome: &Outcome) -> PollState {
    let mut next = prev.clone();
    match outcome {
        Outcome::NotModified { interval } => {
            next.interval_secs = *interval;
            next.failures = 0;
            next.error = None;
        }
        Outcome::Changed {
            last_modified,
            interval,
        } => {
            // A 200 that names no stamp keeps the one there was: dropping it would turn the next
            // request into an unconditional one, and the page of threads into news.
            next.last_modified = last_modified.clone().or_else(|| prev.last_modified.clone());
            next.interval_secs = *interval;
            next.failures = 0;
            next.error = None;
        }
        Outcome::Unread { interval, why } => {
            next.interval_secs = *interval;
            next.failures += 1;
            next.error = Some(why.clone());
        }
        Outcome::Failed(why) => {
            next.failures += 1;
            next.error = Some(why.clone());
        }
    }
    next
}

/// How long to wait before the next round: what GitHub asked (at least a second), else a
/// minute; and while it cannot be reached, doubling that for each failure up to a cap.
pub(super) fn wait(state: &PollState) -> Duration {
    let base = state.interval_secs.unwrap_or(DEFAULT_INTERVAL).max(1);
    let secs = if state.failures == 0 {
        base
    } else {
        base.saturating_mul(1u64 << state.failures.min(16))
            .min(BACKOFF_CAP)
            .max(base)
    };
    Duration::from_secs(secs)
}

/// The cards the news concerns, by position: those whose PR was named by a notification, and
/// every card of a repository that had a check suite finish. When the page of notifications
/// was full there may be news it did not hold (after a restart, or a long gap), so every card
/// is wanted.
pub(super) fn wanted(
    refs: &[PrRef],
    prs: &HashSet<PrRef>,
    check_repos: &HashSet<String>,
    full: bool,
) -> Vec<usize> {
    (0..refs.len())
        .filter(|&i| full || prs.contains(&refs[i]) || check_repos.contains(&refs[i].nwo()))
        .collect()
}

/// Note which PRs have been read once, so a card nobody has read is read on its own only once.
/// A read that failed taught nothing, and the card is tried again.
fn remember(tried: &mut HashSet<PrRef>, asked: Vec<PrRef>, failed: &Option<String>) {
    if failed.is_none() {
        tried.extend(asked);
    }
}

/// What a panic said, for the column's warning.
fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    let said = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_string());
    format!("poll round panicked: {said}")
}

/// One card the poll looks after.
struct Card {
    board: usize,
    task: Task,
    pr: PrRef,
}

/// What `/api/state` says about the poll. No timestamps: the page redraws when the JSON changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollHealth {
    pub active: bool,
    /// `null` when healthy.
    pub error: Option<String>,
}

/// The poll's state, shared with the pages that report its health.
#[derive(Default)]
pub struct PrPoll {
    state: Mutex<PollState>,
    /// The pull request of each branch a session with no task works on, asked in the same round.
    branches: BranchPrs,
    /// The parent GitHub says each task's issue has, asked in the same round.
    parents: IssueParents,
}

/// The boards a round looks after: those with a card on a PR for the cards, and one board of each
/// repository for the branches of its sessions.
#[derive(Default)]
pub struct PollBoards {
    pub cards: Vec<Context>,
    pub branches: Vec<Context>,
    /// The boards with a task on an issue of `HOST`, whose parents the tracker is asked for.
    pub parents: Vec<Context>,
}

/// The repositories a round's notifications name, which makes their branches due.
#[derive(Default)]
struct Named {
    repos: HashSet<String>,
    /// The page was full, so there may be news it did not hold: every repository is named.
    all: bool,
}

impl PrPoll {
    /// What `/api/state` says about the poll. No timestamps: the page redraws when the JSON
    /// changes, and a clock in it would redraw it every poll.
    pub fn health(&self) -> PollHealth {
        PollHealth {
            active: true,
            error: self.lock().error.clone(),
        }
    }

    /// Poll for as long as the process lives. `boards` is asked each round, so a board the
    /// address book gained since is looked after too.
    pub fn run(self: Arc<Self>, boards: impl Fn() -> PollBoards) {
        let mut tried: HashSet<PrRef> = HashSet::new();
        loop {
            // A round that panics is a failed round, not the end of the poll: the thread would
            // die silently and the cards would stay as they were with nothing to say why.
            let round = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.round(&boards(), &mut tried)
            }));
            let pause =
                round.unwrap_or_else(|panic| self.set(&Outcome::Failed(panic_text(&panic))));
            std::thread::sleep(pause);
        }
    }

    /// The state, even if a round panicked while holding it: it is only a cache.
    fn lock(&self) -> std::sync::MutexGuard<'_, PollState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, outcome: &Outcome) -> Duration {
        let mut state = self.lock();
        let next = next_state(&state, outcome);
        // Said once per change: a GitHub that stays unreachable would otherwise fill the log
        // with one line a round.
        if next.error != state.error
            && let Some(why) = &next.error
        {
            // Not `eprintln!`, which panics when stderr is closed: this thread has to outlive
            // whatever the server's log is doing.
            let _ = writeln!(
                std::io::stderr(),
                "adj server: cannot read GitHub's notifications: {why}"
            );
        }
        *state = next;
        wait(&state)
    }

    /// The pull request last found for `branch` of the repository `nwo`, for a session that has
    /// no task, and why the last lookup failed. Read from memory, never from GitHub.
    pub fn branch_pr(
        &self,
        nwo: &str,
        branch: &str,
    ) -> (Option<crate::board::SessionPr>, Option<String>) {
        self.branches.look(nwo, branch)
    }

    /// The parent the tracker last said the issue at `url` has, if it said one. Read from memory,
    /// never from GitHub.
    pub fn issue_parent(&self, url: &str) -> Option<task::TrackerParent> {
        self.parents.look(url)
    }

    /// One round: the cards, the branches of the sessions that have none, then the parents of
    /// the tasks' issues. How long to wait after.
    fn round(&self, boards: &PollBoards, tried: &mut HashSet<PrRef>) -> Duration {
        let mut named = Named::default();
        let pause = self.card_round(&boards.cards, tried, &mut named);
        // Not skipped when no card holds a PR: a session needs its branch looked up whatever
        // the cards say.
        self.branches
            .round(&boards.branches, HOST, &named.repos, named.all);
        self.parents.round(&boards.parents, HOST);
        pause
    }

    /// One round for the cards: ask for news, read what it concerns, apply it.
    fn card_round(
        &self,
        boards: &[Context],
        tried: &mut HashSet<PrRef>,
        named: &mut Named,
    ) -> Duration {
        let mut cards: Vec<Card> = Vec::new();
        for (board, ctx) in boards.iter().enumerate() {
            let tasks = task::candidates(ctx);
            let refs = task::pr_refs(ctx, &tasks);
            for (task, pr) in tasks.into_iter().zip(refs) {
                if let Some(pr) = pr.filter(|r| r.host == HOST) {
                    cards.push(Card { board, task, pr });
                }
            }
        }
        if cards.is_empty() {
            // Nothing to ask GitHub about, so nothing is asked. The error, if there was one,
            // is about cards that are gone.
            let mut state = self.lock();
            state.error = None;
            state.failures = 0;
            return Duration::from_secs(DEFAULT_INTERVAL);
        }
        let (last_modified, reread_due) = {
            let state = self.lock();
            (
                state.last_modified.clone(),
                state
                    .last_reread
                    .is_none_or(|at| at.elapsed() >= STATE_REREAD),
            )
        };
        let deadline = Instant::now() + ROUND_TIMEOUT;
        let news = match gh::notifications(HOST, last_modified.as_deref(), deadline) {
            Ok(news) => news,
            Err(why) => return self.set(&Outcome::Failed(why)),
        };
        let refs: Vec<PrRef> = cards.iter().map(|c| c.pr.clone()).collect();
        let mut read: BTreeSet<usize> = BTreeSet::new();
        // A card nobody has read yet is read once, so a PR opened after the last notification
        // does not wait for the next one to show its state.
        read.extend(
            (0..cards.len())
                .filter(|&i| cards[i].task.pr_status.is_none() && !tried.contains(&refs[i])),
        );
        // With no stamp (the first round after the server started) there is no telling what
        // was missed, so every card is read once.
        let catch_up = last_modified.is_none();
        let (interval, news_state) = match &news {
            Notified::NotModified { interval } => (*interval, None),
            Notified::Changed {
                last_modified,
                interval,
                prs,
                check_repos,
                full,
            } => {
                read.extend(wanted(&refs, prs, check_repos, *full || catch_up));
                named.repos.extend(prs.iter().map(PrRef::nwo));
                named.repos.extend(check_repos.iter().cloned());
                named.all = *full;
                (*interval, Some(last_modified.clone()))
            }
        };
        // A card whose PR could not be read is tried again here too, rather than every round:
        // one that is gone for good would otherwise cost a query a minute.
        if reread_due {
            read.extend((0..cards.len()).filter(|&i| {
                cards[i]
                    .task
                    .pr_status
                    .as_ref()
                    .is_none_or(|status| task::pr_turn(status) != Some(PrTurn::Merged))
            }));
        }
        let mut unread = None;
        if !read.is_empty() {
            let ask: Vec<PrRef> = read.iter().map(|&i| refs[i].clone()).collect();
            let read_now = task::read_prs(&ask, Instant::now() + ROUND_TIMEOUT);
            remember(tried, ask, &read_now.failed);
            // Only a read that could not be made at all is the poll failing. A PR that cannot
            // be found is that card's business, and is reported on it, not on the column.
            unread = read_now.failed;
            let answers = read_now.answers;
            let mut per_board: Vec<(Vec<Task>, Vec<_>)> =
                boards.iter().map(|_| (Vec::new(), Vec::new())).collect();
            let mut cards: Vec<Option<Card>> = cards.into_iter().map(Some).collect();
            for (&i, answer) in read.iter().zip(answers) {
                if let Some(card) = cards[i].take() {
                    per_board[card.board].0.push(card.task);
                    per_board[card.board].1.push(answer);
                }
            }
            for (ctx, (tasks, answers)) in boards.iter().zip(per_board) {
                if !tasks.is_empty() {
                    task::apply(ctx, tasks, answers);
                }
            }
        }
        // Only once the pending cards were read: a failed read must be tried again soon.
        if reread_due && unread.is_none() {
            self.lock().last_reread = Some(Instant::now());
        }
        self.set(&match (unread, news_state) {
            (Some(why), _) => Outcome::Unread { interval, why },
            (None, Some(last_modified)) => Outcome::Changed {
                last_modified,
                interval,
            },
            (None, None) => Outcome::NotModified { interval },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(owner: &str, repo: &str, number: u64) -> PrRef {
        task::pr_ref(
            &format!("https://github.com/{owner}/{repo}/pull/{number}"),
            None,
        )
        .unwrap()
    }

    fn state(last_modified: Option<&str>, interval: Option<u64>, failures: u32) -> PollState {
        PollState {
            last_modified: last_modified.map(str::to_string),
            interval_secs: interval,
            failures,
            ..PollState::default()
        }
    }

    #[test]
    fn a_304_keeps_the_stamp_and_takes_the_new_interval() {
        let prev = state(Some("x"), Some(60), 2);
        let next = next_state(&prev, &Outcome::NotModified { interval: Some(5) });
        assert_eq!(next.last_modified.as_deref(), Some("x"));
        assert_eq!(
            (next.interval_secs, next.failures, next.error),
            (Some(5), 0, None)
        );
    }

    #[test]
    fn a_200_without_a_stamp_keeps_the_one_there_was() {
        let prev = state(Some("x"), Some(60), 0);
        let next = next_state(
            &prev,
            &Outcome::Changed {
                last_modified: None,
                interval: Some(60),
            },
        );
        assert_eq!(next.last_modified.as_deref(), Some("x"));
    }

    #[test]
    fn a_200_replaces_the_stamp() {
        let prev = state(Some("x"), Some(60), 0);
        let next = next_state(
            &prev,
            &Outcome::Changed {
                last_modified: Some("y".to_string()),
                interval: Some(30),
            },
        );
        assert_eq!(next.last_modified.as_deref(), Some("y"));
        assert_eq!(next.interval_secs, Some(30));
    }

    #[test]
    fn news_that_could_not_be_read_does_not_advance_the_stamp() {
        let prev = state(Some("x"), Some(60), 0);
        let next = next_state(
            &prev,
            &Outcome::Unread {
                interval: Some(60),
                why: "boom".to_string(),
            },
        );
        assert_eq!(next.last_modified.as_deref(), Some("x"));
        assert_eq!((next.failures, next.error.as_deref()), (1, Some("boom")));
        let healed = next_state(&next, &Outcome::NotModified { interval: Some(60) });
        assert_eq!((healed.failures, healed.error), (0, None));
    }

    #[test]
    fn the_wait_is_what_github_asked_a_minute_when_it_did_not_and_never_under_a_second() {
        assert_eq!(wait(&state(None, None, 0)), Duration::from_secs(60));
        assert_eq!(wait(&state(None, Some(30), 0)), Duration::from_secs(30));
        assert_eq!(wait(&state(None, Some(0), 0)), Duration::from_secs(1));
        assert_eq!(wait(&state(None, Some(3600), 0)), Duration::from_secs(3600));
    }

    #[test]
    fn failures_double_the_wait_up_to_a_cap_and_keep_the_stamp() {
        let waits: Vec<u64> = (0..6)
            .map(|failures| wait(&state(Some("x"), None, failures)).as_secs())
            .collect();
        assert_eq!(waits, [60, 120, 240, 480, 900, 900]);
        // A long interval GitHub asked for is not cut short by the cap.
        assert_eq!(wait(&state(None, Some(3600), 3)).as_secs(), 3600);
        // Many failures do not overflow.
        assert_eq!(wait(&state(None, Some(60), u32::MAX)).as_secs(), 900);
        let failed = next_state(
            &state(Some("x"), None, 0),
            &Outcome::Failed("no".to_string()),
        );
        assert_eq!(failed.last_modified.as_deref(), Some("x"));
    }

    #[test]
    fn the_cards_wanted_are_those_the_news_names_or_whose_repository_had_a_suite_finish() {
        let refs = [
            pr("acme", "widget", 7),
            pr("acme", "widget", 8),
            pr("acme", "gadget", 1),
        ];
        let prs = HashSet::from([pr("Acme", "Widget", 7)]);
        assert_eq!(wanted(&refs, &prs, &HashSet::new(), false), [0]);
        let repos = HashSet::from(["acme/gadget".to_string()]);
        assert_eq!(wanted(&refs, &prs, &repos, false), [0, 2]);
        let both = HashSet::from(["acme/widget".to_string()]);
        assert_eq!(wanted(&refs, &HashSet::new(), &both, false), [0, 1]);
        assert!(wanted(&refs, &HashSet::new(), &HashSet::new(), false).is_empty());
    }

    #[test]
    fn a_failed_read_does_not_count_as_a_card_having_been_tried() {
        let mut tried = HashSet::new();
        let asked = vec![pr("acme", "widget", 7)];
        remember(&mut tried, asked.clone(), &Some("rate limit".to_string()));
        assert!(tried.is_empty());
        remember(&mut tried, asked, &None);
        assert!(tried.contains(&pr("acme", "widget", 7)));
    }

    #[test]
    fn a_full_page_of_notifications_wants_every_card() {
        let refs = [pr("acme", "widget", 7), pr("acme", "gadget", 1)];
        assert_eq!(
            wanted(&refs, &HashSet::new(), &HashSet::new(), true),
            [0, 1]
        );
    }

    #[test]
    fn a_panic_is_said_in_the_polls_own_words() {
        let caught = std::panic::catch_unwind(|| panic!("boom")).unwrap_err();
        assert_eq!(panic_text(&caught), "poll round panicked: boom");
        let caught =
            std::panic::catch_unwind(|| panic!("{}", String::from("formatted"))).unwrap_err();
        assert_eq!(panic_text(&caught), "poll round panicked: formatted");
    }
}
