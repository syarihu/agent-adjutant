//! What a transport says: the answers `cli`, `mcp` and `board_http` give in the same words.
//!
//! Each of the three reads input, calls an operation and words the result. `wording` holds the
//! answers two or more of them give the same way, so none of them has to name another to say it.

pub mod wording;

#[cfg(test)]
mod tests;
