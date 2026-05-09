use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::state::{Profile, PptLimits};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedProfile {
    pub name: String,
    pub platform_profile: Profile,
    pub ppt: PptLimits,
    pub fan_curve: Vec<(f32, f32)>,
    pub fan_hysteresis: u8,
}

/// Upserts `profile` into `list` by name (replaces if name already exists).
pub fn upsert(list: &mut Vec<SavedProfile>, profile: SavedProfile) {
    if let Some(existing) = list.iter_mut().find(|p| p.name == profile.name) {
        *existing = profile;
    } else {
        list.push(profile);
    }
}

pub fn load() -> Vec<SavedProfile> {
    let path = config_dir().join("profiles.json");
    if !path.exists() {
        return Vec::new();
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save(profiles: &[SavedProfile]) {
    let path = config_dir().join("profiles.json");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(profiles) {
        let _ = fs::write(&path, json);
    }
}

/// Persists the name of the last-applied saved profile.
pub fn save_active(name: &str) {
    let path = config_dir().join("active-profile");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, name);
}

/// Returns the last-applied saved profile name, if any.
pub fn load_active() -> Option<String> {
    let s = fs::read_to_string(config_dir().join("active-profile")).ok()?;
    let name = s.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        });
    base.join("strixctl")
}
