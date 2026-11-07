//! Running repository lifecycle hooks when a worktree is removed.

use serde_json::Value;
use std::path::Path;

use crate::infra::template::{Sub, render};

/// One `onWorktreeRemove` command and how it ended, as the error text when it failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRun {
    pub command: String,
    pub result: Result<(), String>,
}

impl serde::Serialize for HookRun {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("HookRun", 3)?;
        s.serialize_field("command", &self.command)?;
        match &self.result {
            Ok(()) => {
                s.serialize_field("ok", &true)?;
                s.serialize_field("error", &None::<String>)?;
            }
            Err(e) => {
                s.serialize_field("ok", &false)?;
                s.serialize_field("error", &Some(e))?;
            }
        }
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for HookRun {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct HookRunHelper {
            command: String,
            ok: bool,
            #[serde(default)]
            error: Option<String>,
        }
        let helper = HookRunHelper::deserialize(deserializer)?;
        Ok(HookRun {
            command: helper.command,
            result: if helper.ok {
                Ok(())
            } else {
                Err(helper.error.unwrap_or_default())
            },
        })
    }
}

fn run_hooks_for_key(key: &str, nwo: &str, main: &str, worktree: &str, name: &str) -> Vec<HookRun> {
    let hooks = crate::kernel::config::resolve_config(nwo)
        .ok()
        .and_then(|resolved| resolved.config)
        .and_then(|config| config.get(key).cloned())
        .and_then(|hooks| hooks.as_array().cloned())
        .unwrap_or_default();
    hooks
        .iter()
        .filter_map(Value::as_str)
        .map(|template| {
            let command = render(
                template,
                &[
                    ("worktree", Sub::Quoted(worktree)),
                    ("name", Sub::Quoted(name)),
                ],
            );
            let ran = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(Path::new(main))
                .output();
            match ran {
                Ok(out) if out.status.success() => HookRun {
                    command,
                    result: Ok(()),
                },
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    let error = match stderr.is_empty() {
                        true => format!("exited with {}", out.status),
                        false => stderr,
                    };
                    HookRun {
                        command,
                        result: Err(error),
                    }
                }
                Err(e) => HookRun {
                    command,
                    result: Err(e.to_string()),
                },
            }
        })
        .collect()
}

/// The repository's `onWorktreeRemove` commands, run in the main checkout after the removal
/// with `{worktree}` and `{name}` filled in, as the hub does. Read from the config now, not
/// from the settings the server started with. A hook that fails is reported and does not
/// undo anything.
pub fn run_remove_hooks(nwo: &str, main: &str, worktree: &str, name: &str) -> Vec<HookRun> {
    run_hooks_for_key("onWorktreeRemove", nwo, main, worktree, name)
}

/// The repository's `postCreate` commands, run in the main checkout after creating the
/// worktree with `{worktree}` and `{name}` filled in. Read from the config now, not from
/// the settings the server started with. A hook that fails is reported and does not undo
/// anything.
pub fn run_create_hooks(nwo: &str, main: &str, worktree: &str, name: &str) -> Vec<HookRun> {
    run_hooks_for_key("postCreate", nwo, main, worktree, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_run_serializes_as_command_ok_error() {
        let ok_hook = HookRun {
            command: "touch foo".to_string(),
            result: Ok(()),
        };
        let ok_json = serde_json::to_value(&ok_hook).unwrap();
        assert_eq!(
            ok_json,
            serde_json::json!({
                "command": "touch foo",
                "ok": true,
                "error": null,
            })
        );

        let err_hook = HookRun {
            command: "false".to_string(),
            result: Err("exited with 1".to_string()),
        };
        let err_json = serde_json::to_value(&err_hook).unwrap();
        assert_eq!(
            err_json,
            serde_json::json!({
                "command": "false",
                "ok": false,
                "error": "exited with 1",
            })
        );
    }
}
