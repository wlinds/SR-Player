// Utility functions for image processing and date parsing
//
// This module provides helper functions for:
// - Downloading images from URLs
// - Converting image bytes to Slint Image format
// - Parsing Sveriges Radio's date format

use chrono::{DateTime, Utc};
use chrono_tz::Europe::Stockholm;
use slint::Image;

pub fn parse_sr_date_to_time(date_str: &str) -> String {
    if let Some(ms_str) = date_str
        .strip_prefix("/Date(")
        .and_then(|s| s.strip_suffix(")/"))
    {
        if let Ok(ms) = ms_str.parse::<i64>() {
            return DateTime::from_timestamp_millis(ms)
                .map(|dt: DateTime<Utc>| {
                    let stockholm_time = dt.with_timezone(&Stockholm);
                    stockholm_time.format("%H:%M").to_string()
                })
                .unwrap_or_else(|| String::from("--:--"));
        }
    }
    String::from("--:--")
}

/// Download image bytes from a URL
///
/// Returns raw bytes which are Send-safe for use across threads.
/// Call this in spawn_blocking, then use bytes_to_slint_image on the main thread.
pub fn download_image_bytes(url: &str) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }

    match reqwest::blocking::get(url) {
        Ok(response) => response.bytes().ok().map(|b| b.to_vec()),
        Err(e) => {
            eprintln!("Failed to download image from {}: {}", url, e);
            None
        }
    }
}

/// Convert raw image bytes to Slint Image format
///
/// MUST be called on the main thread (Slint Image is not Send).
pub fn bytes_to_slint_image(bytes: Vec<u8>) -> Option<Image> {
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            let buffer =
                slint::SharedPixelBuffer::clone_from_slice(&rgba.into_raw(), width, height);
            Some(Image::from_rgba8(buffer))
        }
        Err(e) => {
            eprintln!("Failed to decode image: {}", e);
            None
        }
    }
}
