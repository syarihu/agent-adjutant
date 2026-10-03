/// What the built-in close prints when it found the tab and asked it to close.
///
/// The same device `wake` uses, for the same reason: walking every window and matching no
/// tty is a script that ran to the end and exited 0, indistinguishable from the one that
/// reached a session — so the one path that reached a session says so out loud.
///
/// What it does *not* say is that the tab is gone. iTerm2 can be set to confirm closing a
/// session with a process still in it, and cancelling that dialog is not an AppleScript
/// error: the script goes on and prints this. So the marker rules out "there was no such
/// tab", and nothing more. Whether the worker actually died is a question about the
/// process, and it is asked one layer up, by the caller that acts on the answer.
pub(super) const CLOSED_MARKER: &str = "adjutant:closed";

// ── waking a running session ─────────────────────────────────────────

/// Type a line into the tab a running session is sitting in.
///
/// iTerm2's `write text` is the built-in because it is the one mechanism that actually
/// reaches an interactive agent: the message goes to its prompt exactly as if the person
/// had typed it. Any other terminal with a scripting interface can be dropped in as a
/// template.
/// What the built-in wake prints when it actually typed into a session. Its absence is the
/// only way to tell "typed into the tab" from "walked every window and found no such tab":
/// both are a script that ran to the end and exited 0.
pub(super) const WOKE_MARKER: &str = "adjutant:woke";

/// Did the command actually reach a session? A template answers for itself with its exit
/// status; the built-in has to say so out loud, because running to the end having found
/// nothing is indistinguishable from running to the end having typed a line.
pub(super) fn woke(built_in: bool, output: &str) -> bool {
    !built_in || output.contains(WOKE_MARKER)
}

/// How long to wait between typing the line and pressing Enter. The two writes have to
/// reach the agent as two reads; one that is busy and reads both at once sees a burst again.
pub(super) const WAKE_ENTER_DELAY: &str = "0.2";
