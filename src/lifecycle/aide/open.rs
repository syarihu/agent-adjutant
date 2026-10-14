use std::path::Path;

use crate::infra::clock::now_secs;
use crate::infra::terminal::Performed;
use crate::kernel::config::{Settings, resolve_config};
use crate::kernel::identity::AIDE_CONFIG_KEY;
use crate::lifecycle::hub::hub_startable;
use crate::registry::{self, AideClaim, AideRecord};

use super::launch_aide;

/// What opening the aide came to.
#[derive(Debug, PartialEq)]
pub enum AideOpened {
    /// A new aide was started, and what opening its window did.
    Started {
        record: AideRecord,
        description: String,
    },
    /// One is running already.
    Running(AideRecord),
    /// One was started a moment ago and has not reported yet.
    Starting(AideRecord),
}

/// Why the aide was not opened.
#[derive(Debug, PartialEq)]
pub enum AideRefused {
    /// The terminal is not tmux, which is the only terminal a window can be shown from.
    NotStartable,
    Failed(String),
}

/// Open the aide unless one is running or starting: claim a session, then open a window that
/// runs it. Checked before anything is written, so a refusal leaves no record behind.
pub fn open_aide(root: &Path) -> Result<AideOpened, AideRefused> {
    let settings = resolve_config(AIDE_CONFIG_KEY)
        .map_err(AideRefused::Failed)?
        .settings;
    open_aide_with(
        root,
        &settings,
        now_secs(),
        &registry::claim_aide,
        &|session| launch_aide(root, &settings, session),
    )
}

/// `open_aide`, with the claim and the launch handed in.
pub(crate) fn open_aide_with(
    root: &Path,
    settings: &Settings,
    now: i64,
    claim: &dyn Fn(&Path, i64) -> Result<AideClaim, String>,
    launch: &dyn Fn(&str) -> Result<Performed, String>,
) -> Result<AideOpened, AideRefused> {
    if !hub_startable(&settings.terminal) {
        return Err(AideRefused::NotStartable);
    }
    match claim(root, now).map_err(AideRefused::Failed)? {
        AideClaim::Running(record) => Ok(AideOpened::Running(record)),
        AideClaim::Starting(record) => Ok(AideOpened::Starting(record)),
        AideClaim::Claimed(record) => match launch(&record.session_id) {
            Ok(performed) => Ok(AideOpened::Started {
                record,
                description: performed.description,
            }),
            Err(why) => {
                // The claim is taken back so the next open does not wait out the start grace.
                let cleared = registry::clear_aide_if(root, &record.session_id);
                Err(AideRefused::Failed(match cleared {
                    Ok(()) => why,
                    Err(e) => format!("{why} ({e})"),
                }))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Recorded, claim_aide};

    fn tmux() -> (crate::testing::Sandbox, Settings) {
        let sandbox =
            crate::testing::Sandbox::new(r#"{"defaults":{"terminal":{"preset":"tmux"}}}"#);
        let settings = resolve_config("@aide").unwrap().settings;
        (sandbox, settings)
    }

    fn performed() -> Performed {
        Performed {
            description: "opened".to_string(),
            script: String::new(),
            ran: true,
            screen: false,
        }
    }

    #[test]
    fn a_terminal_that_is_not_tmux_is_refused_before_anything_is_written() {
        let sandbox =
            crate::testing::Sandbox::new(r#"{"defaults":{"terminal":{"preset":"iterm2"}}}"#);
        let settings = resolve_config("@aide").unwrap().settings;
        let root = sandbox.state();
        let out = open_aide_with(&root, &settings, 100, &claim_aide, &|_| {
            panic!("no window may be opened")
        });
        assert_eq!(out, Err(AideRefused::NotStartable));
        assert_eq!(registry::aide_record(&root), Recorded::Absent);
    }

    #[test]
    fn a_failed_launch_takes_its_claim_back() {
        let (_sandbox, settings) = tmux();
        let dir = tempfile::tempdir().unwrap();
        let out = open_aide_with(dir.path(), &settings, 100, &claim_aide, &|_| {
            Err("no window".to_string())
        });
        assert_eq!(out, Err(AideRefused::Failed("no window".to_string())));
        assert_eq!(registry::aide_record(dir.path()), Recorded::Absent);
    }

    #[test]
    fn a_launch_that_works_leaves_the_claim_and_a_second_open_finds_it_starting() {
        let (_sandbox, settings) = tmux();
        let dir = tempfile::tempdir().unwrap();
        let Ok(AideOpened::Started {
            record,
            description,
        }) = open_aide_with(
            dir.path(),
            &settings,
            100,
            &claim_aide,
            &|_| Ok(performed()),
        )
        else {
            panic!("expected a start");
        };
        assert_eq!(description, "opened");
        let again = open_aide_with(dir.path(), &settings, 101, &claim_aide, &|_| {
            panic!("a second window")
        });
        assert_eq!(again, Ok(AideOpened::Starting(record)));
    }

    #[test]
    fn a_start_that_could_not_be_confirmed_keeps_the_claim() {
        let (_sandbox, settings) = tmux();
        let dir = tempfile::tempdir().unwrap();
        let out = open_aide_with(dir.path(), &settings, 100, &claim_aide, &|session| {
            super::super::launch::launch_aide_with(
                dir.path(),
                &settings,
                session,
                &|_| Ok((performed(), Some("%1".to_string()))),
                &|_| Err("tmux did not answer".to_string()),
            )
        });
        assert!(matches!(out, Ok(AideOpened::Started { .. })), "{out:?}");
        assert!(matches!(
            registry::aide_record(dir.path()),
            Recorded::Found(_)
        ));
    }
}
