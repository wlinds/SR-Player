// Favorites Persistence Module
//
// Handles saving and loading favorite channels and programs to/from disk.
// Uses a JSON file in the user's config directory for cross-platform support.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Data structure for storing favorites
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Favorites {
    pub channel_ids: HashSet<i32>,
    pub program_ids: HashSet<i32>,
}

impl Favorites {
    /// Get the path to the favorites file
    fn get_favorites_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut path| {
            path.push("sr-player");
            path.push("favorites.json");
            path
        })
    }

    /// Load favorites from disk
    pub fn load() -> Self {
        let Some(path) = Self::get_favorites_path() else {
            eprintln!("Could not determine config directory");
            return Self::default();
        };

        if !path.exists() {
            return Self::default();
        }

        match fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(favorites) => {
                    println!("Loaded favorites from {:?}", path);
                    favorites
                }
                Err(e) => {
                    eprintln!("Failed to parse favorites file: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("Failed to read favorites file: {}", e);
                Self::default()
            }
        }
    }

    /// Save favorites to disk
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::get_favorites_path() else {
            return Err("Could not determine config directory".to_string());
        };

        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Err(format!("Failed to create config directory: {}", e));
                }
            }
        }

        // Serialize and write to file
        match serde_json::to_string_pretty(self) {
            Ok(json) => match fs::write(&path, json) {
                Ok(()) => {
                    println!("Saved favorites to {:?}", path);
                    Ok(())
                }
                Err(e) => Err(format!("Failed to write favorites file: {}", e)),
            },
            Err(e) => Err(format!("Failed to serialize favorites: {}", e)),
        }
    }

    /// Check if a channel is favorited
    pub fn is_channel_favorite(&self, channel_id: i32) -> bool {
        self.channel_ids.contains(&channel_id)
    }

    /// Check if a program is favorited
    pub fn is_program_favorite(&self, program_id: i32) -> bool {
        self.program_ids.contains(&program_id)
    }

    /// Toggle a channel's favorite status
    pub fn toggle_channel(&mut self, channel_id: i32) -> bool {
        if self.channel_ids.contains(&channel_id) {
            self.channel_ids.remove(&channel_id);
            false
        } else {
            self.channel_ids.insert(channel_id);
            true
        }
    }

    /// Toggle a program's favorite status
    pub fn toggle_program(&mut self, program_id: i32) -> bool {
        if self.program_ids.contains(&program_id) {
            self.program_ids.remove(&program_id);
            false
        } else {
            self.program_ids.insert(program_id);
            true
        }
    }
}
