// File-based audio player with seeking support
//
// This player downloads the entire episode first, then plays from memory.
// Unlike streaming, this allows seeking to any position.
//
// Architecture:
// 1. Download entire file to memory
// 2. Use rodio::Decoder with Cursor (supports seeking)
// 3. Rodio's Sink with seek support

use anyhow::{Context, Result};
use bytes::Bytes;
use log::info;
use minimp3::{Decoder as Mp3Decoder, Frame};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

// Custom MP3 source with fast byte-level seeking
struct FastMp3Source {
    decoder: Mp3Decoder<Cursor<Vec<u8>>>,
    current_frame: Option<Frame>,
    sample_offset: usize,
    // Fade-in to avoid clicking noise
    fade_in_samples: usize,
    samples_played: usize,
}

impl FastMp3Source {
    fn new(data: Vec<u8>, seek_seconds: f32) -> Result<Self> {
        let file_size = data.len();

        // First pass: scan first 50 frames to get average frame size and sample rate
        let mut temp_decoder = Mp3Decoder::new(Cursor::new(data.clone()));
        let first_frame = temp_decoder
            .next_frame()
            .context("Failed to decode first frame")?;
        let sample_rate = first_frame.sample_rate as f32;
        let channels = first_frame.channels;

        // Calculate fade-in duration: 8ms
        let fade_in_samples = (sample_rate * 0.008) as usize * channels;

        // Scan first 50 frames to estimate average frame size
        let mut frame_count = 1; // We already decoded first frame
        let max_sample_frames = 50;

        while frame_count < max_sample_frames {
            match temp_decoder.next_frame() {
                Ok(_) => frame_count += 1,
                Err(_) => break,
            }
        }

        // Estimate average bytes per frame
        // After decoding N frames, we can estimate how many bytes were consumed
        // Typical MP3 frame: 417-626 bytes at 128-192 kbps
        let avg_bytes_per_frame = if frame_count > 0 {
            // Rough estimate: first N frames should be near start of file
            // For 128 kbps CBR MP3: ~417 bytes/frame
            // For 192 kbps CBR MP3: ~626 bytes/frame
            // Let's use a conservative 500 bytes/frame average
            500
        } else {
            500
        };

        // Calculate how many frames to skip
        // MP3 frames are typically 26ms (1152 samples at 44.1kHz)
        let samples_per_frame = 1152;
        let frames_per_second = sample_rate / samples_per_frame as f32;
        let target_frame = (seek_seconds * frames_per_second) as usize;

        // Calculate approximate byte offset
        let target_byte_pos = (target_frame * avg_bytes_per_frame).min(file_size - 1000) as u64;

        info!(
            "Fast MP3 seek: seeking to {}s (frame ~{}, byte pos ~{})",
            seek_seconds, target_frame, target_byte_pos
        );

        // Create new decoder starting from estimated position
        // Slice the data from target position to allow minimp3 to sync
        let seek_data = &data[target_byte_pos as usize..];
        let mut decoder = Mp3Decoder::new(Cursor::new(seek_data.to_vec()));

        // Sync to next valid frame (minimp3 handles this automatically)
        let first_valid_frame = decoder
            .next_frame()
            .context("Failed to find valid frame after seek")?;

        info!(
            "Synced to valid MP3 frame at sample rate {}",
            first_valid_frame.sample_rate
        );

        Ok(Self {
            decoder,
            current_frame: Some(first_valid_frame),
            sample_offset: 0,
            fade_in_samples,
            samples_played: 0,
        })
    }
}

impl Iterator for FastMp3Source {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Return samples from current frame
            if let Some(frame) = &self.current_frame {
                if self.sample_offset < frame.data.len() {
                    let mut sample = frame.data[self.sample_offset];

                    // Apply fade-in to avoid clicking (linear fade over 8ms)
                    if self.samples_played < self.fade_in_samples {
                        let fade_factor = self.samples_played as f32 / self.fade_in_samples as f32;
                        sample = (sample as f32 * fade_factor) as i16;
                        self.samples_played += 1;
                    }

                    self.sample_offset += 1;
                    return Some(sample);
                }
            }

            // Load next frame
            match self.decoder.next_frame() {
                Ok(frame) => {
                    self.current_frame = Some(frame);
                    self.sample_offset = 0;
                }
                Err(_) => return None,
            }
        }
    }
}

impl Source for FastMp3Source {
    fn current_frame_len(&self) -> Option<usize> {
        self.current_frame.as_ref().map(|f| f.data.len())
    }

    fn channels(&self) -> u16 {
        self.current_frame
            .as_ref()
            .map(|f| f.channels as u16)
            .unwrap_or(2)
    }

    fn sample_rate(&self) -> u32 {
        self.current_frame
            .as_ref()
            .map(|f| f.sample_rate as u32)
            .unwrap_or(44100)
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

// File-based audio player for podcast episodes
pub struct FilePlayer {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Sink>>,
    audio_data: Arc<Mutex<Option<Bytes>>>, // Store downloaded audio data
}

impl FilePlayer {
    // Create a new file-based player
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) =
            OutputStream::try_default().context("Failed to create audio output stream")?;

        let sink = Sink::try_new(&stream_handle).context("Failed to create audio sink")?;

        Ok(Self {
            _stream: stream,
            _stream_handle: stream_handle,
            sink: Arc::new(Mutex::new(sink)),
            audio_data: Arc::new(Mutex::new(None)),
        })
    }

    // Stop playback
    pub async fn stop(&self) {
        let sink = self.sink.lock().await;
        sink.stop();
        drop(sink);

        let mut data = self.audio_data.lock().await;
        *data = None;
    }

    // Pause playback
    pub fn pause(&self) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.pause();
        }
    }

    // Resume playback
    pub fn resume(&self) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.play();
        }
    }

    // Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.set_volume(volume);
        }
    }

    // Play from bytes at a specific position (FAST with minimp3!)
    pub async fn play_from_bytes_at_position(&self, data: Bytes, position: f32) -> Result<()> {
        info!("Fast seeking to {}s using minimp3", position);

        // Store audio data for future seeks
        {
            let mut audio_data = self.audio_data.lock().await;
            *audio_data = Some(data.clone());
        }

        // Get current volume before stopping
        let volume = {
            let sink = self.sink.lock().await;
            sink.volume()
        };

        // Fade out current audio quickly to avoid click (8ms)
        {
            let sink = self.sink.lock().await;
            let initial_volume = sink.volume();

            // Quick fade-out over 8ms
            for i in (0..40).rev() {
                sink.set_volume(initial_volume * (i as f32 / 40.0));
                tokio::time::sleep(Duration::from_micros(200)).await;
            }
        }

        // Create fast MP3 source on blocking thread (frame skipping is fast!)
        let data_vec = data.to_vec();
        let source = tokio::task::spawn_blocking(move || FastMp3Source::new(data_vec, position))
            .await
            .context("Spawn task failed")?
            .context("Failed to create MP3 source")?;

        info!("MP3 source ready, starting playback at {}s", position);

        // Replace the entire sink to start fresh
        let sink = self.sink.lock().await;

        sink.stop();
        drop(sink);

        // Create new sink
        let new_sink = Sink::try_new(&self._stream_handle).context("Failed to create new sink")?;
        new_sink.set_volume(volume);
        new_sink.append(source);
        new_sink.play();

        let mut sink = self.sink.lock().await;
        *sink = new_sink;

        info!("Playing from {}s", position);
        Ok(())
    }
}
