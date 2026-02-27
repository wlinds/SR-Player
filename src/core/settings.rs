// Settings Persistence Module
//
// Handles saving and loading user settings to/from disk.
// Uses a JSON file in the user's config directory for cross-platform support.

use crate::localization::Language;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Data structure for storing user settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub language: Language,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::default(), // Swedish
        }
    }
}

impl Settings {
    /// Get the path to the settings file
    fn get_settings_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut path| {
            path.push("sr-player");
            path.push("settings.json");
            path
        })
    }

    /// Load settings from disk
    pub fn load() -> Self {
        let Some(path) = Self::get_settings_path() else {
            eprintln!("Could not determine config directory");
            return Self::default();
        };

        if !path.exists() {
            return Self::default();
        }

        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(settings) => {
                    println!("Loaded settings from {:?}", path);
                    settings
                }
                Err(e) => {
                    eprintln!("Failed to parse settings file: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to read settings file: {}", e);
                Self::default()
            }
        }
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::get_settings_path() else {
            return Err("Could not determine config directory".to_string());
        };

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!("Failed to create config directory: {}", e)
                })?;
            }
        }

        // Serialize and write
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        fs::write(&path, json)
            .map_err(|e| format!("Failed to write settings file: {}", e))?;

        println!("Saved settings to {:?}", path);
        Ok(())
    }
}
