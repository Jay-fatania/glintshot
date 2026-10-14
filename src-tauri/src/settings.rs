//! User settings, stored as JSON in the app's config folder.
//!
//! Missing or unknown fields fall back to defaults, so older and newer
//! settings files keep loading. A corrupt file is ignored (defaults are used)
//! and left untouched until the user saves new settings.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::Shortcut;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EnterAction {
    /// Save the PNG and copy it.
    Save,
    /// Copy to the clipboard only.
    Copy,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub screenshot_dir: PathBuf,
    pub recording_dir: PathBuf,
    /// Hotkeys in global-hotkey syntax, e.g. "ctrl+alt+KeyS".
    pub screenshot_hotkey: String,
    pub record_hotkey: String,
    /// Mirrors the OS login item; the OS setting is the source of truth.
    pub start_at_login: bool,
    pub enter_action: EnterAction,
    pub recording_fps: u32,
}

pub const FPS_CHOICES: [u32; 2] = [30, 60];

impl Settings {
    pub fn defaults(pictures: &Path, videos: &Path) -> Self {
        Self {
            screenshot_dir: pictures.join("Glintshot"),
            recording_dir: videos.join("Glintshot"),
            screenshot_hotkey: "ctrl+alt+KeyS".into(),
            record_hotkey: "ctrl+alt+KeyR".into(),
            start_at_login: false,
            enter_action: EnterAction::Save,
            recording_fps: 60,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, dir) in [("Screenshot", &self.screenshot_dir), ("Recording", &self.recording_dir)] {
            if !dir.is_absolute() {
                return Err(format!("{name} folder must be a full path."));
            }
        }
        let screenshot = parse_hotkey(&self.screenshot_hotkey).map_err(|e| format!("Screenshot hotkey: {e}"))?;
        let record = parse_hotkey(&self.record_hotkey).map_err(|e| format!("Record hotkey: {e}"))?;
        if screenshot == record {
            return Err("The screenshot and record hotkeys must be different.".into());
        }
        if !FPS_CHOICES.contains(&self.recording_fps) {
            return Err(format!("Recording frame rate must be one of {FPS_CHOICES:?}."));
        }
        Ok(())
    }
}

/// Parses a hotkey and requires at least one modifier, so a plain letter
/// can't hijack normal typing everywhere.
pub fn parse_hotkey(keys: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = keys.parse().map_err(|e| format!("\"{keys}\" is not a valid hotkey ({e})"))?;
    if shortcut.mods.is_empty() {
        return Err(format!("\"{keys}\" needs at least one of Ctrl, Alt, Shift or Super."));
    }
    Ok(shortcut)
}

/// Loads settings, filling any missing fields from `defaults`.
pub fn load(path: &Path, defaults: Settings) -> Settings {
    let Ok(text) = fs::read_to_string(path) else {
        return defaults;
    };
    let Ok(serde_json::Value::Object(stored)) = serde_json::from_str::<serde_json::Value>(&text) else {
        eprintln!("glintshot: ignoring unreadable settings file {}", path.display());
        return defaults;
    };
    let mut merged = match serde_json::to_value(&defaults) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => return defaults,
    };
    for (key, value) in stored {
        if merged.contains_key(&key) {
            merged.insert(key, value);
        }
    }
    match serde_json::from_value::<Settings>(serde_json::Value::Object(merged)) {
        Ok(settings) if settings.validate().is_ok() => settings,
        _ => {
            eprintln!("glintshot: settings file {} has invalid values; using defaults", path.display());
            defaults
        }
    }
}

/// Writes settings atomically (temp file, then rename), readable only by the user.
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("could not write settings: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("could not save settings: {e}"))
}

/// Makes sure an output folder exists. Only a folder Glintshot creates is
/// made private (mode 700); an existing folder the user picked, such as the
/// Desktop, keeps its permissions.
pub fn ensure_dir(dir: &Path) -> Result<(), String> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Settings {
        Settings::defaults(Path::new("/home/u/Pictures"), Path::new("/home/u/Videos"))
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("glintshot-settings-{}-{name}", std::process::id()))
    }

    #[test]
    fn defaults_are_valid() {
        defaults().validate().unwrap();
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(load(&temp_file("missing.json"), defaults()), defaults());
    }

    #[test]
    fn partial_file_keeps_other_defaults_and_ignores_unknown_keys() {
        let path = temp_file("partial.json");
        fs::write(&path, r#"{"recording_fps": 30, "future_option": true}"#).unwrap();
        let loaded = load(&path, defaults());
        fs::remove_file(&path).unwrap();
        assert_eq!(loaded.recording_fps, 30);
        assert_eq!(loaded.screenshot_hotkey, defaults().screenshot_hotkey);
    }

    #[test]
    fn corrupt_or_invalid_file_gives_defaults() {
        let path = temp_file("corrupt.json");
        fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path, defaults()), defaults());
        fs::write(&path, r#"{"recording_fps": 45}"#).unwrap();
        assert_eq!(load(&path, defaults()), defaults());
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = temp_file("roundtrip.json");
        let mut settings = defaults();
        settings.enter_action = EnterAction::Copy;
        settings.record_hotkey = "ctrl+shift+KeyR".into();
        save(&path, &settings).unwrap();
        let loaded = load(&path, defaults());
        fs::remove_file(&path).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn rejects_bad_settings() {
        let mut s = defaults();
        s.record_hotkey = s.screenshot_hotkey.clone();
        assert!(s.validate().is_err(), "duplicate hotkeys");

        let mut s = defaults();
        s.screenshot_hotkey = "KeyS".into();
        assert!(s.validate().is_err(), "no modifier");

        let mut s = defaults();
        s.screenshot_dir = "relative/dir".into();
        assert!(s.validate().is_err(), "relative folder");
    }
}
