//! The board: the server that answers the page, the read model it answers from, the jobs it
//! runs in the background beside answering requests, and the operations a person runs from
//! it. Names `infra`, `kernel`, `registry`, `mail`, `task`, `gate`, `jules` and `lifecycle`,
//! and nothing above them.

pub mod jobs;
