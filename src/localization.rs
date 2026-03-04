// Language selection and configuration for Slint's bundled translations
//
// Translations are handled by Slint's @tr() macro and bundled at compile time.
// This module only defines the Language enum for settings persistence.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    English,
    Swedish,
    Arabic,
}

impl Language {
    /// Get the language code for use with slint::select_bundled_translation()
    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Swedish => "sv",
            Language::Arabic => "ar",
        }
    }

    /// Parse a language code back into a Language enum
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::English),
            "sv" => Some(Language::Swedish),
            "ar" => Some(Language::Arabic),
            _ => None,
        }
    }
}

impl Default for Language {
    fn default() -> Self {
        Language::Swedish
    }
}
