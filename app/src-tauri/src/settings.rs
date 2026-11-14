//! The app's own settings file, `settings.json` in the app config directory.
//!
//! Keys: `adjPath` (the `adj` to run), `translucency` ("off" | "weak" | "strong"), `shortcut`
//! (the global shortcut; `null` or "" turns it off) and `zoom` (the board's page zoom, 1 is 100%).
//! Read leniently: a key that is missing or malformed falls back to its default, so a hand edit
//! never keeps the app from starting.

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

/// The page zoom steps the menu and its shortcuts move through, as a browser's do. A value written
/// by hand between two of them is kept as it is, and the next zoom in or out goes to the next step.
pub const ZOOM_STEPS: [f64; 13] = [
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];
pub const MIN_ZOOM: f64 = ZOOM_STEPS[0];
pub const MAX_ZOOM: f64 = ZOOM_STEPS[ZOOM_STEPS.len() - 1];
/// How close two zooms must be to count as the same step, so that 0.6700001 is not a step below 0.67.
const ZOOM_EPSILON: f64 = 1e-6;

/// A zoom read from the file: anything that is not a positive finite number is 100%, and a number
/// outside the steps is brought back to the nearest end of them.
pub fn clamp_zoom(zoom: f64) -> f64 {
    if zoom.is_finite() && zoom > 0.0 {
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    } else {
        1.0
    }
}

/// The next step above the zoom, or the largest step when there is none.
pub fn zoom_in(zoom: f64) -> f64 {
    ZOOM_STEPS
        .into_iter()
        .find(|step| *step > zoom + ZOOM_EPSILON)
        .unwrap_or(MAX_ZOOM)
}

/// The next step below the zoom, or the smallest step when there is none.
pub fn zoom_out(zoom: f64) -> f64 {
    ZOOM_STEPS
        .into_iter()
        .rev()
        .find(|step| *step < zoom - ZOOM_EPSILON)
        .unwrap_or(MIN_ZOOM)
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub adj_path: Option<String>,
    pub translucency: Level,
    pub shortcut: Option<String>,
    pub zoom: f64,
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
    let zoom = map
        .get("zoom")
        .and_then(Value::as_f64)
        .map_or(1.0, clamp_zoom);
    Settings {
        adj_path,
        translucency,
        shortcut,
        zoom,
    }
}

/// Each save writes its one key back and leaves every other key as the person wrote it.
pub fn save_translucency(app: &AppHandle, level: Level) -> Result<(), String> {
    save_key(app, "translucency", Value::String(level.key().into()))
}

pub fn save_zoom(app: &AppHandle, zoom: f64) -> Result<(), String> {
    let value = serde_json::Number::from_f64(zoom).ok_or("倍率が数値ではありません")?;
    save_key(app, "zoom", Value::Number(value))
}

/// Writes one key back into the file and leaves every other key (and any the app does not know)
/// as the person wrote it.
fn save_key(app: &AppHandle, key: &str, value: Value) -> Result<(), String> {
    let file = path(app).ok_or("設定ディレクトリが見つかりません")?;
    let mut map = read_map(app)?;
    map.insert(key.into(), value);
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
        assert_eq!(s.zoom, 1.0);
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

    #[test]
    fn zoom_is_read_and_a_bad_one_falls_back() {
        assert_eq!(parse(r#"{"zoom": 1.25}"#).zoom, 1.25);
        assert_eq!(parse(r#"{"zoom": 2}"#).zoom, 2.0);
        // Between two steps: kept as it is.
        assert_eq!(parse(r#"{"zoom": 1.2}"#).zoom, 1.2);
        // Outside the steps: the nearest end.
        assert_eq!(parse(r#"{"zoom": 10}"#).zoom, MAX_ZOOM);
        assert_eq!(parse(r#"{"zoom": 0.1}"#).zoom, MIN_ZOOM);
        // Not a positive number: 100%.
        for bad in [r#""big""#, "0", "-1", "null", "true", "[1]"] {
            assert_eq!(parse(&format!(r#"{{"zoom": {bad}}}"#)).zoom, 1.0, "{bad}");
        }
        assert_eq!(clamp_zoom(f64::NAN), 1.0);
        assert_eq!(clamp_zoom(f64::INFINITY), 1.0);
    }

    #[test]
    fn zoom_moves_through_the_steps() {
        assert_eq!(zoom_in(1.0), 1.1);
        assert_eq!(zoom_out(1.0), 0.9);
        assert_eq!(zoom_in(0.67), 0.75);
        assert_eq!(zoom_out(0.75), 0.67);
        // From a value between two steps, to the next one on that side.
        assert_eq!(zoom_in(1.2), 1.25);
        assert_eq!(zoom_out(1.2), 1.1);
        // At the ends, it stays.
        assert_eq!(zoom_in(MAX_ZOOM), MAX_ZOOM);
        assert_eq!(zoom_out(MIN_ZOOM), MIN_ZOOM);
        // Every step is reached going up and going down.
        let mut zoom = MIN_ZOOM;
        let mut up = vec![zoom];
        while zoom < MAX_ZOOM {
            zoom = zoom_in(zoom);
            up.push(zoom);
        }
        assert_eq!(up, ZOOM_STEPS);
        let mut down = vec![zoom];
        while zoom > MIN_ZOOM {
            zoom = zoom_out(zoom);
            down.push(zoom);
        }
        down.reverse();
        assert_eq!(down, ZOOM_STEPS);
    }
}
