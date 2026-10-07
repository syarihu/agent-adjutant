//! A task's history, read when the page opens it.

use serde::Serialize;

use crate::board::Server;
use crate::gate;

/// A task's history as `/api/tasks/{id}/history` sends it: the records keep their diffs.
#[derive(Debug, Clone, Serialize)]
pub struct TaskHistory {
    pub answered: Vec<gate::Gate>,
    pub records: Vec<gate::Gate>,
}

/// Everything one task's gates left behind, for its panel: the gates a person answered
/// (`answered`) and the records its worker kept (`records`), each oldest first.
///
/// Asked for by the page when it opens the view rather than joined into `/api/state`: the
/// archive only grows, and reading all of it on every poll would cost more each day. The
/// records are here as well as on the task in `/api/state` because a finished task's are not
/// there, and a review is meant to stay readable after the work is done.
pub fn task_history(server: &Server, id: &str) -> TaskHistory {
    history_of(
        id,
        gate::list(
            &server.ctx.state,
            &server.ctx.repo.slug,
            gate::Shelf::Answered,
        ),
        gate::list(
            &server.ctx.state,
            &server.ctx.repo.slug,
            gate::Shelf::Record,
        ),
    )
}

pub fn history_of(id: &str, answered: Vec<gate::Gate>, records: Vec<gate::Gate>) -> TaskHistory {
    let mine = |g: &gate::Gate| g.task.as_deref() == Some(id);
    TaskHistory {
        answered: answered.into_iter().filter(mine).collect(),
        records: records.into_iter().filter(mine).collect(),
    }
}
