use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::state::PptLimits;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedProfile {
    pub name: String,
    /// None means this profile does not touch PPT limits.
    pub ppt: Option<PptLimits>,
    /// None means this profile does not touch the fan curve.
    pub fan_curve: Option<Vec<(f32, f32)>>,
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
    let path = profiles_path();
    if !path.exists() {
        return Vec::new();
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save(profiles: &[SavedProfile]) {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(profiles) {
        let _ = fs::write(&path, json);
    }
}

fn profiles_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config")
        });
    config_dir.join("strixctrl").join("profiles.json")
}
