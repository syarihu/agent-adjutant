//! Which of the person's columns a gate puts its task in, and whether a card's pull request
//! waits on the person: sent on `/api/state` as `humanCol` and `waitsOnPerson`, which the page's
//! `humanColOf` reads.

use serde::Serialize;

use crate::board::jobs::JulesSeen;
use crate::gate;
use crate::task;

/// One of the person's columns, written as the page's column id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HumanCol {
    Dispatch,
    Plan,
    Diff,
    Verify,
    PrReview,
    Question,
}

impl HumanCol {
    /// The column an open gate of this kind puts its task in, whatever else the task says.
    pub fn of_gate(kind: gate::Kind) -> Self {
        match kind {
            gate::Kind::Dispatch | gate::Kind::Issue => HumanCol::Dispatch,
            gate::Kind::Plan => HumanCol::Plan,
            gate::Kind::Diff => HumanCol::Diff,
            gate::Kind::Verify | gate::Kind::Result => HumanCol::Verify,
            gate::Kind::Relay => HumanCol::PrReview,
            gate::Kind::Question => HumanCol::Question,
        }
    }
}

/// Whether a task's pull request is the person's ball, for a task with no open gate (a gate
/// decides by its own `HumanCol`). `phase` is the task's worker row: `None` with none,
/// `Some(phase)` with one. `jules` is the board's last answer about the task's session.
///
/// A finished task waits on nothing. While Jules is working the ball is Jules's, whatever the
/// PR says; once it is not, the PR's turn decides, unless the session failed, when the task is
/// decided as a worker task is: a worker in a phase other than `pr` or `pr-bots` is at work,
/// otherwise the PR's turn decides, and a turn that says nothing leaves it to the phase (`pr`)
/// or, with no worker row, the status (`pr`).
pub fn waits_on_person(
    task: &task::Task,
    phase: Option<Option<&str>>,
    jules: Option<&JulesSeen>,
) -> bool {
    if matches!(task.status, task::Status::Done | task::Status::Cancelled) {
        return false;
    }
    // A blank `pr` or `julesSession` cannot be on a record, so `is_some` is all there is to ask.
    if task.jules_session.is_some() && task.pr.is_some() {
        match jules {
            Some(JulesSeen::Found { working: true, .. }) => return false,
            Some(JulesSeen::Found { state, .. }) if state == "FAILED" => {}
            _ => return task::jules_pr_waits_on_person(task.pr_status.as_ref()),
        }
    }
    task::pr_waits_on_person(
        task.status,
        task.pr.is_some(),
        task.pr_status.as_ref(),
        phase,
    )
}
