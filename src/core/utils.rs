// Utility functions for image processing, date parsing, and audio conversion

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Stockholm;
use slint::Image;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::sample::Sample;

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

/// Shared async HTTP client for image downloads (connection pooling)
fn shared_image_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("SR-Player/3.0")
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Failed to create image HTTP client")
    })
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

/// Download image bytes from a URL using shared async client
pub async fn fetch_image_bytes(url: String) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }
    match shared_image_client().get(&url).send().await {
        Ok(response) => response.bytes().await.ok().map(|b| b.to_vec()),
        Err(e) => {
            eprintln!("Failed to download image from {}: {}", url, e);
            None
        }
    }
}

// ============================================================================
// AUDIO BUFFER CONVERSION UTILITIES
// ============================================================================

/// Generic helper to interleave audio buffer channels with sample conversion
///
/// This function handles converting Symphonia audio buffers to the desired output format
/// by interleaving channels (e.g., [L R L R L R...] for stereo).
///
/// # Arguments
/// * `buf` - The audio buffer from AudioBufferRef (Cow type)
/// * `convert` - Function to convert sample type T to output type O
///
/// # Returns
/// Vec<O> with samples interleaved by channel
pub fn interleave_audio_buffer<T, O>(
    buf: &symphonia::core::audio::AudioBuffer<T>,
    convert: impl Fn(T) -> O,
) -> Vec<O>
where
    T: Sample + Copy,
{
    let num_channels = buf.spec().channels.count();
    let frames = buf.frames();

    if num_channels == 1 {
        // Mono: just convert samples
        buf.chan(0).iter().map(|&s| convert(s)).collect()
    } else {
        // Multi-channel: interleave L R L R L R...
        let mut interleaved = Vec::with_capacity(frames * num_channels);
        for frame in 0..frames {
            for ch in 0..num_channels {
                interleaved.push(convert(buf.chan(ch)[frame]));
            }
        }
        interleaved
    }
}

/// Convert Symphonia AudioBufferRef to i16 samples (for gapless streaming)
///
/// Handles all Symphonia sample formats and converts to i16 with proper scaling.
/// Channels are interleaved: [L R L R L R...] for stereo.
pub fn audio_buffer_to_i16(decoded: &AudioBufferRef) -> Vec<i16> {
    match decoded {
        AudioBufferRef::S16(buf) => interleave_audio_buffer(buf, |sample| sample),
        AudioBufferRef::F32(buf) => {
            interleave_audio_buffer(buf, |sample| (sample * 32767.0) as i16)
        }
        AudioBufferRef::S32(buf) => interleave_audio_buffer(buf, |sample| (sample >> 16) as i16),
        AudioBufferRef::U8(buf) => {
            interleave_audio_buffer(buf, |sample| (sample as i16 - 128) << 8)
        }
        AudioBufferRef::U16(buf) => {
            interleave_audio_buffer(buf, |sample| (sample as i32 - 32768) as i16)
        }
        AudioBufferRef::U32(buf) => {
            interleave_audio_buffer(buf, |sample| ((sample >> 16) as i32 - 32768) as i16)
        }
        AudioBufferRef::S8(buf) => interleave_audio_buffer(buf, |sample| (sample as i16) << 8),
        AudioBufferRef::F64(buf) => {
            interleave_audio_buffer(buf, |sample| (sample * 32767.0) as i16)
        }
        AudioBufferRef::U24(buf) => {
            interleave_audio_buffer(buf, |sample| ((sample.inner() >> 8) as i32 - 32768) as i16)
        }
        AudioBufferRef::S24(buf) => {
            interleave_audio_buffer(buf, |sample| (sample.inner() >> 8) as i16)
        }
    }
}

/// Convert Symphonia AudioBufferRef to f32 samples (for file playback)
///
/// Handles all Symphonia sample formats and converts to normalized f32 [-1.0, 1.0].
/// Channels are interleaved: [L R L R L R...] for stereo.
pub fn audio_buffer_to_f32(decoded: &AudioBufferRef) -> Vec<f32> {
    match decoded {
        AudioBufferRef::F32(buf) => interleave_audio_buffer(buf, |sample| sample),
        AudioBufferRef::F64(buf) => interleave_audio_buffer(buf, |sample| sample as f32),
        AudioBufferRef::S16(buf) => interleave_audio_buffer(buf, |sample| sample as f32 / 32768.0),
        AudioBufferRef::S32(buf) => {
            interleave_audio_buffer(buf, |sample| sample as f32 / 2147483648.0)
        }
        AudioBufferRef::S8(buf) => interleave_audio_buffer(buf, |sample| sample as f32 / 128.0),
        AudioBufferRef::U8(buf) => {
            interleave_audio_buffer(buf, |sample| (sample as f32 - 128.0) / 128.0)
        }
        AudioBufferRef::U16(buf) => {
            interleave_audio_buffer(buf, |sample| (sample as f32 - 32768.0) / 32768.0)
        }
        AudioBufferRef::U32(buf) => {
            interleave_audio_buffer(buf, |sample| (sample as f32 - 2147483648.0) / 2147483648.0)
        }
        AudioBufferRef::U24(buf) => interleave_audio_buffer(buf, |sample| {
            let val = sample.inner();
            (val as f32 - 8388608.0) / 8388608.0
        }),
        AudioBufferRef::S24(buf) => {
            interleave_audio_buffer(buf, |sample| sample.inner() as f32 / 8388608.0)
        }
    }
}

// ============================================================================
// TIME PARSING UTILITIES
// ============================================================================

/// Parse a time string in "HH:MM" format to hours and minutes
///
/// # Arguments
/// * `time_str` - Time string like "09:30" or "14:00"
///
/// # Returns
/// Some((hours, minutes)) if valid, None if parse fails
pub fn parse_hm_time(time_str: &str) -> Option<(i32, i32)> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() == 2 {
        let hours = parts[0].parse::<i32>().ok()?;
        let minutes = parts[1].parse::<i32>().ok()?;
        Some((hours, minutes))
    } else {
        None
    }
}

/// Calculate program playback position and duration from time strings
///
/// Handles overnight programs (e.g., 23:00 - 02:00) by adding 24 hours.
///
/// # Arguments
/// * `start_time` - Program start time "HH:MM"
/// * `end_time` - Program end time "HH:MM"
///
/// # Returns
/// (position_seconds, duration_seconds) based on current Stockholm time
pub fn calculate_program_position(start_time: &str, end_time: &str) -> Option<(f32, f32)> {
    let (start_h, start_m) = parse_hm_time(start_time)?;
    let (end_h, end_m) = parse_hm_time(end_time)?;

    // Get current Stockholm time
    let now = Utc::now().with_timezone(&Stockholm);
    let current_h = now.hour() as i32;
    let current_m = now.minute() as i32;
    let current_s = now.second() as i32;

    // Calculate minutes from midnight
    let start_minutes = start_h * 60 + start_m;
    let mut end_minutes = end_h * 60 + end_m;

    // Handle overnight programs (e.g., 23:00 - 02:00)
    if end_minutes < start_minutes {
        end_minutes += 24 * 60;
    }

    let current_minutes = current_h * 60 + current_m;

    // Calculate position
    let mut elapsed_minutes = current_minutes - start_minutes;
    if elapsed_minutes < 0 {
        elapsed_minutes += 24 * 60; // Handle wrap-around
    }

    let duration_minutes = end_minutes - start_minutes;
    let position = (elapsed_minutes * 60 + current_s) as f32;
    let duration = (duration_minutes * 60) as f32;

    Some((position, duration))
}
