//! The app's own settings file, `settings.json` in the app config directory.
//!
//! Keys: `adjPath` (the `adj` to run), `translucency` ("off" | "weak" | "strong") and `shortcut`
//! (the global shortcut; `null` or "" turns it off). Read leniently: a key that is missing or
//! malformed falls back to its default, so a hand edit never keeps the app from starting.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};

pub const DEFAULT_SHORTCUT: &str = "Control+Alt+Super+B";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Off,
    Weak,
    Strong,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Off, Level::Weak, Level::Strong];

    pub fn key(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Weak => "weak",
            Level::Strong => "strong",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Off => "なし",
            Level::Weak => "弱",
            Level::Strong => "強",
        }
    }

    /// How opaque the board's big surfaces stay; 1 is the browser's look.
    pub fn alpha(self) -> f64 {
        match self {
            Level::Off => 1.0,
            Level::Weak => 0.82,
            Level::Strong => 0.55,
        }
    }

    pub fn from_key(key: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|level| level.key() == key)
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub adj_path: Option<String>,
    pub translucency: Level,
    pub shortcut: Option<String>,
}

fn path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("settings.json"))
}

/// The file as an object. A file that is not there is an empty one; one that is there and cannot
/// be read or is not an object is an error, so that a save never overwrites what it could not read.
fn read_map(app: &AppHandle) -> Result<Map<String, Value>, String> {
    let Some(file) = path(app) else {
        return Ok(Map::new());
    };
    parse_file(&file)
}

fn parse_file(file: &Path) -> Result<Map<String, Value>, String> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(format!("{}: not a JSON object", file.display())),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

pub fn load(app: &AppHandle) -> Settings {
    from_map(&read_map(app).unwrap_or_default())
}

fn from_map(map: &Map<String, Value>) -> Settings {
    let adj_path = map
        .get("adjPath")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let translucency = map
        .get("translucency")
        .and_then(Value::as_str)
        .and_then(Level::from_key)
        .unwrap_or(Level::Weak);
    let shortcut = match map.get("shortcut") {
        None => Some(DEFAULT_SHORTCUT.to_owned()),
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Some(Value::String(_)) | Some(Value::Null) => None,
        Some(_) => Some(DEFAULT_SHORTCUT.to_owned()),
    };
    Settings {
        adj_path,
        translucency,
        shortcut,
    }
}

/// Writes the translucency back into the file and leaves every other key (and any the app does
/// not know) as the person wrote it.
pub fn save_translucency(app: &AppHandle, level: Level) -> Result<(), String> {
    let file = path(app).ok_or("設定ディレクトリが見つかりません")?;
    let mut map = read_map(app)?;
    map.insert("translucency".into(), Value::String(level.key().into()));
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(map)).map_err(|e| e.to_string())?;
    // Written beside the file and renamed over it, so that a crash mid-write cannot leave half a file.
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, text + "\n").map_err(|e| e.to_string())?;
    fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Settings {
        from_map(
            serde_json::from_str::<Value>(text)
                .unwrap()
                .as_object()
                .unwrap(),
        )
    }

    #[test]
    fn defaults_when_empty() {
        let s = parse("{}");
        assert_eq!(s.translucency, Level::Weak);
        assert_eq!(s.shortcut.as_deref(), Some(DEFAULT_SHORTCUT));
        assert_eq!(s.adj_path, None);
    }

    #[test]
    fn null_or_empty_shortcut_turns_it_off() {
        assert_eq!(parse(r#"{"shortcut": null}"#).shortcut, None);
        assert_eq!(parse(r#"{"shortcut": ""}"#).shortcut, None);
        assert_eq!(
            parse(r#"{"shortcut": "Control+Alt+K"}"#)
                .shortcut
                .as_deref(),
            Some("Control+Alt+K")
        );
    }

    #[test]
    fn an_unreadable_file_is_an_error_and_a_missing_one_is_empty() {
        let dir =
            std::env::temp_dir().join(format!("adjutant-app-settings-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(parse_file(&dir.join("missing.json")).unwrap().is_empty());
        let broken = dir.join("broken.json");
        fs::write(&broken, "{ not json").unwrap();
        assert!(parse_file(&broken).is_err());
        let array = dir.join("array.json");
        fs::write(&array, "[1]").unwrap();
        assert!(parse_file(&array).is_err());
        let fine = dir.join("fine.json");
        fs::write(&fine, r#"{"adjPath": "/x"}"#).unwrap();
        assert_eq!(parse_file(&fine).unwrap().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn malformed_values_fall_back() {
        let s = parse(r#"{"translucency": "lots", "adjPath": 3}"#);
        assert_eq!(s.translucency, Level::Weak);
        assert_eq!(s.adj_path, None);
    }
}
