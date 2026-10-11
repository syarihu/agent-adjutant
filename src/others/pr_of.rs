//! The PR a person or a command names: a pull request URL, or a record's key `owner/repo#N`.

use crate::task::{PrRef, pr_ref};

/// The host the records are about: the sync searches this one only.
const HOST: &str = "github.com";

/// The PR `text` names, or `None` for anything else. Nothing is guessed: a bare number has no
/// repository, and a URL on another host is not a record of ours.
pub fn pr_of(text: &str) -> Option<PrRef> {
    let text = text.trim();
    let pr = match text.split_once('#') {
        // `owner/repo#N`, and not a URL whose fragment happens to follow.
        Some((nwo, number)) if !text.contains("://") => pr_ref(number, Some((HOST, nwo)))?,
        _ => pr_ref(text, None)?,
    };
    (pr.host == HOST).then_some(pr)
}
