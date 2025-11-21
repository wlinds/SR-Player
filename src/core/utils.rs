// Utility functions for image processing, date parsing, and audio conversion

use chrono::{DateTime, Utc};
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

pub async fn fetch_image_bytes(url: String) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }
    tokio::task::spawn_blocking(move || download_image_bytes(&url))
        .await
        .ok()
        .flatten()
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
    buf: &std::borrow::Cow<symphonia::core::audio::AudioBuffer<T>>,
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
        AudioBufferRef::S16(buf) => interleave_audio_buffer(&buf, |sample| sample),
        AudioBufferRef::F32(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample * 32767.0) as i16)
        }
        AudioBufferRef::S32(buf) => interleave_audio_buffer(&buf, |sample| (sample >> 16) as i16),
        AudioBufferRef::U8(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample as i16 - 128) << 8)
        }
        AudioBufferRef::U16(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample as i32 - 32768) as i16)
        }
        AudioBufferRef::U32(buf) => {
            interleave_audio_buffer(&buf, |sample| ((sample >> 16) as i32 - 32768) as i16)
        }
        AudioBufferRef::S8(buf) => interleave_audio_buffer(&buf, |sample| (sample as i16) << 8),
        AudioBufferRef::F64(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample * 32767.0) as i16)
        }
        AudioBufferRef::U24(buf) => {
            interleave_audio_buffer(&buf, |sample| ((sample.inner() >> 8) as i32 - 32768) as i16)
        }
        AudioBufferRef::S24(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample.inner() >> 8) as i16)
        }
    }
}

/// Convert Symphonia AudioBufferRef to f32 samples (for file playback)
///
/// Handles all Symphonia sample formats and converts to normalized f32 [-1.0, 1.0].
/// Channels are interleaved: [L R L R L R...] for stereo.
pub fn audio_buffer_to_f32(decoded: &AudioBufferRef) -> Vec<f32> {
    match decoded {
        AudioBufferRef::F32(buf) => interleave_audio_buffer(&buf, |sample| sample),
        AudioBufferRef::F64(buf) => interleave_audio_buffer(&buf, |sample| sample as f32),
        AudioBufferRef::S16(buf) => interleave_audio_buffer(&buf, |sample| sample as f32 / 32768.0),
        AudioBufferRef::S32(buf) => {
            interleave_audio_buffer(&buf, |sample| sample as f32 / 2147483648.0)
        }
        AudioBufferRef::S8(buf) => interleave_audio_buffer(&buf, |sample| sample as f32 / 128.0),
        AudioBufferRef::U8(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample as f32 - 128.0) / 128.0)
        }
        AudioBufferRef::U16(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample as f32 - 32768.0) / 32768.0)
        }
        AudioBufferRef::U32(buf) => {
            interleave_audio_buffer(&buf, |sample| (sample as f32 - 2147483648.0) / 2147483648.0)
        }
        AudioBufferRef::U24(buf) => interleave_audio_buffer(&buf, |sample| {
            let val = sample.inner() as u32;
            (val as f32 - 8388608.0) / 8388608.0
        }),
        AudioBufferRef::S24(buf) => {
            interleave_audio_buffer(&buf, |sample| sample.inner() as f32 / 8388608.0)
        }
    }
}
