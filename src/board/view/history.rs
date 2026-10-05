//! A task's history, read when the page opens it.

use serde_json::{Value, json};

use crate::board::Server;
use crate::gate;

/// Everything one task's gates left behind, for its full view: the gates a person answered
/// (`answered`) and the records its worker kept (`records`), each oldest first.
///
/// Asked for by the page when it opens the view rather than joined into `/api/state`: the
/// archive only grows, and reading all of it on every poll would cost more each day. The
/// records are here as well as on the task in `/api/state` because a finished task's are not
/// there, and a review is meant to stay readable after the work is done.
pub fn task_history(server: &Server, path: &str) -> Result<Value, String> {
    let id = history_id(path).ok_or("no such task")?;
    Ok(history_of(
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
    ))
}

/// The task id in `/api/tasks/{id}/history`, when there is exactly one.
pub fn history_id(path: &str) -> Option<&str> {
    path.strip_prefix("/api/tasks/")
        .and_then(|rest| rest.strip_suffix("/history"))
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

pub fn history_of(id: &str, answered: Vec<gate::Gate>, records: Vec<gate::Gate>) -> Value {
    let mine = |g: &gate::Gate| g.task.as_deref() == Some(id);
    json!({
        "answered": answered.into_iter().filter(mine).collect::<Vec<_>>(),
        "records": records.into_iter().filter(mine).collect::<Vec<_>>(),
    })
}
