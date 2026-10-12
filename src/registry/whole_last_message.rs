//! Reading the whole last message of an agent session.

use super::store::{agent_message_path, read_agent_message};
use super::*;

/// The last message of one session as the expanded view shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct LastMessageRead {
    pub session_id: String,
    /// The row's `lastMessageAt`.
    pub at: Option<i64>,
    pub text: Option<String>,
    /// Whether `text` is all of what the agent said: false when only the row's cut text is left.
    pub whole: bool,
}

/// Whether a row's `lastMessage` was cut, for a row that has no side file to say so. A cut text
/// ends in an ellipsis and runs `trim_end` before it, so it can be a bit shorter than the limit.
pub fn looks_cut(text: &str) -> bool {
    let chars = text.chars().count();
    chars > LAST_MESSAGE_CHARS || (text.ends_with('…') && chars >= LAST_MESSAGE_CHARS * 9 / 10)
}

/// The whole last message of `session_id`, or what the row keeps when the side file is not there,
/// is unreadable or belongs to another turn. `None` when there is no row; an error when the id is
/// not a file name or the row cannot be read (a failure is not "no row"). An unreadable side file
/// is only a fallback: the row still answers.
pub fn whole_last_message(
    root: &Path,
    session_id: &str,
) -> Result<Option<LastMessageRead>, String> {
    if !valid_session_id(session_id) {
        return Err(format!("not a usable session id: {session_id:?}"));
    }
    let row = match agent_session(root, session_id) {
        Recorded::Found(row) => row,
        Recorded::Absent => return Ok(None),
        Recorded::Unreadable => {
            return Err(format!("cannot read the row of session {session_id}"));
        }
    };
    let read = |text: Option<String>, whole: bool| {
        Ok(Some(LastMessageRead {
            session_id: row.session_id.clone(),
            at: row.last_message_at,
            text,
            whole,
        }))
    };
    let Some(kept) = row.last_message.as_deref() else {
        return read(None, true);
    };
    if let Recorded::Found(side) = read_agent_message(&agent_message_path(root, session_id))
        && Some(side.at) == row.last_message_at
    {
        return read(Some(side.text), true);
    }
    let text = cut_last_message(kept, LAST_MESSAGE_CHARS);
    let whole = !looks_cut(&text);
    read(Some(text), whole)
}

#[cfg(test)]
mod tests;
