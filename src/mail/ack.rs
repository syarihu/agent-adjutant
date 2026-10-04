use super::*;

/// Move one message out of the way. The hub calls this once it has filed the report.
///
/// The archive is where a mishandled report is found again, so an archived name is never
/// reused: two messages acked in the same second are two files, not one file and one loss.
///
/// The message is *taken* before it is filed, not after. Filing first and unlinking second
/// is the order that loses one: two acks of a name each file a copy, the first unlink frees
/// the name, a send in the same second claims it, and the second unlink deletes that new
/// message — which nothing filed. So the inbox name is claimed in one step, by renaming it
/// onto a name only this call knows. A second acker's rename finds nothing and says so.
///
/// Between the two steps the message is real but hidden, which is a state this has to be
/// able to come back from: the name it was taken from is written into the holding name, and
/// `list` puts back anything it finds there that is too old to be in flight.
pub fn ack(slug: &str, name: &str) -> Result<PathBuf, String> {
    let inbox = inbox_dir(slug);
    let from = safe_join(&inbox, name)?;
    let dir = archive_dir(slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let held = hold(&inbox, name)?;
    if let Err(e) = std::fs::rename(&from, &held) {
        let _ = std::fs::remove_file(&held);
        return match e.kind() {
            std::io::ErrorKind::NotFound => Err(format!("no message called {name} is waiting")),
            _ => Err(format!("cannot move {}: {e}", from.display())),
        };
    }
    // From here the message exists only under the holding name, so it is unlinked from
    // there only once something else holds it. An ack that cannot file *and* cannot put
    // back leaves it where the sweep below will find it, rather than deleting it to keep
    // the directory tidy.
    match claim_link(&held, &dir, |seq| numbered(name, seq)) {
        Ok(to) => {
            let _ = std::fs::remove_file(&held);
            Ok(to)
        }
        Err(e) => match claim_link(&held, &inbox, |seq| numbered(name, seq)) {
            Ok(back) => {
                let _ = std::fs::remove_file(&held);
                Err(format!(
                    "{e} — the message is back in the inbox as {}",
                    back.file_name().unwrap_or_default().to_string_lossy()
                ))
            }
            Err(_) => Err(format!(
                "{e} — the message is held at {} and will be put back",
                held.display()
            )),
        },
    }
}
