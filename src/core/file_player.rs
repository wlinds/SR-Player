// File-based audio player with seeking support
//
// This player downloads the entire episode first, then plays from memory.
// Unlike streaming, this allows seeking to any position.
//
// Architecture:
// 1. Download entire file to memory
// 2. Use Symphonia decoder with Cursor (supports MP3, AAC, and more)
// 3. Rodio's Sink with seek support

use anyhow::{Context, Result};
use bytes::Bytes;
use log::info;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// Custom audio source with fast byte-level seeking (supports MP3, AAC, etc)
struct FastAudioSource {
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    format_reader: Box<dyn symphonia::core::formats::FormatReader>,
    track_id: u32,
    current_samples: Vec<f32>,
    sample_offset: usize,
    sample_rate: u32,
    channels: u16,
    // Fade-in to avoid clicking noise
    fade_in_samples: usize,
    samples_played: usize,
}

impl FastAudioSource {
    fn new(data: Vec<u8>, seek_seconds: f32) -> Result<Self> {
        // Create media source from bytes
        let cursor = Cursor::new(data.clone());
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

        // Probe the format (auto-detect MP3, AAC, etc)
        let hint = Hint::new();
        // Don't set extension - let Symphonia auto-detect

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .context("Failed to probe audio format")?;

        let mut format_reader = probed.format;

        // Find the first audio track
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .context("No audio tracks found")?;

        let track_id = track.id;
        let codec_params = &track.codec_params;

        let sample_rate = codec_params
            .sample_rate
            .context("Sample rate not found")? as u32;

        let channels = codec_params
            .channels
            .context("Channels not found")?
            .count() as u16;

        info!(
            "Detected audio format: sample_rate={}, channels={}",
            sample_rate, channels
        );

        // Calculate fade-in duration: 8ms
        let fade_in_samples = (sample_rate as f32 * 0.008) as usize * channels as usize;

        // Create decoder for the track
        let decoder_opts = DecoderOptions::default();
        let mut decoder = symphonia::default::get_codecs()
            .make(codec_params, &decoder_opts)
            .context("Failed to create decoder")?;

        // If seeking is requested, try to seek (time-based seeking)
        if seek_seconds > 0.0 {
            let seek_to_ts = (seek_seconds * sample_rate as f32) as u64;

            // Try to seek to the target timestamp
            // Symphonia will handle frame alignment automatically
            if let Err(e) = format_reader.seek(
                symphonia::core::formats::SeekMode::Accurate,
                symphonia::core::formats::SeekTo::TimeStamp { ts: seek_to_ts, track_id },
            ) {
                info!("Seek not supported or failed: {}, starting from beginning", e);
            } else {
                info!("Seeked to approximately {}s", seek_seconds);
            }
        }

        // Decode first packet to get initial samples
        let mut current_samples = Vec::new();

        // Try to decode first packet
        loop {
            let packet = match format_reader.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(_)) => break,
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                    continue;
                }
                Err(e) => return Err(anyhow::anyhow!("Failed to read packet: {}", e)),
            };

            // Only decode packets for our track
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    // Convert to f32 samples
                    current_samples = Self::convert_audio_buffer(&decoded);
                    break;
                }
                Err(SymphoniaError::DecodeError(e)) => {
                    info!("Decode error (skipping): {}", e);
                    continue;
                }
                Err(e) => return Err(anyhow::anyhow!("Failed to decode: {}", e)),
            }
        }

        Ok(Self {
            decoder,
            format_reader,
            track_id,
            current_samples,
            sample_offset: 0,
            sample_rate,
            channels,
            fade_in_samples,
            samples_played: 0,
        })
    }

    // Convert Symphonia AudioBufferRef to f32 samples
    fn convert_audio_buffer(decoded: &AudioBufferRef) -> Vec<f32> {
        match decoded {
            AudioBufferRef::F32(buf) => {
                // Interleave channels
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        samples.push(buf.chan(ch_idx)[frame_idx]);
                    }
                }
                samples
            }
            AudioBufferRef::U8(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert u8 [0, 255] to f32 [-1.0, 1.0]
                        let sample = (buf.chan(ch_idx)[frame_idx] as f32 - 128.0) / 128.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::U16(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert u16 [0, 65535] to f32 [-1.0, 1.0]
                        let sample = (buf.chan(ch_idx)[frame_idx] as f32 - 32768.0) / 32768.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::U32(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert u32 [0, 4294967295] to f32 [-1.0, 1.0]
                        let sample = (buf.chan(ch_idx)[frame_idx] as f32 - 2147483648.0) / 2147483648.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::S8(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert s8 [-128, 127] to f32 [-1.0, 1.0]
                        let sample = buf.chan(ch_idx)[frame_idx] as f32 / 128.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::S16(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert s16 [-32768, 32767] to f32 [-1.0, 1.0]
                        let sample = buf.chan(ch_idx)[frame_idx] as f32 / 32768.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::S32(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert s32 [-2147483648, 2147483647] to f32 [-1.0, 1.0]
                        let sample = buf.chan(ch_idx)[frame_idx] as f32 / 2147483648.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::F64(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        samples.push(buf.chan(ch_idx)[frame_idx] as f32);
                    }
                }
                samples
            }
            AudioBufferRef::U24(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert u24 to f32 [-1.0, 1.0]
                        // u24 value range [0, 16777215]
                        let val = buf.chan(ch_idx)[frame_idx].inner() as u32;
                        let sample = (val as f32 - 8388608.0) / 8388608.0;
                        samples.push(sample);
                    }
                }
                samples
            }
            AudioBufferRef::S24(buf) => {
                let mut samples = Vec::new();
                let num_frames = buf.frames();
                let num_channels = buf.spec().channels.count();

                for frame_idx in 0..num_frames {
                    for ch_idx in 0..num_channels {
                        // Convert s24 to f32 [-1.0, 1.0]
                        // s24 value range [-8388608, 8388607]
                        let val = buf.chan(ch_idx)[frame_idx].inner();
                        let sample = val as f32 / 8388608.0;
                        samples.push(sample);
                    }
                }
                samples
            }
        }
    }
}

impl Iterator for FastAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Return samples from current buffer
            if self.sample_offset < self.current_samples.len() {
                let mut sample = self.current_samples[self.sample_offset];

                // Apply fade-in to avoid clicking (linear fade over 8ms)
                if self.samples_played < self.fade_in_samples {
                    let fade_factor = self.samples_played as f32 / self.fade_in_samples as f32;
                    sample *= fade_factor;
                    self.samples_played += 1;
                }

                self.sample_offset += 1;
                return Some(sample);
            }

            // Load next packet
            let packet = match self.format_reader.next_packet() {
                Ok(packet) => packet,
                Err(_) => return None,
            };

            // Only decode packets for our track
            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    self.current_samples = Self::convert_audio_buffer(&decoded);
                    self.sample_offset = 0;
                }
                Err(_) => continue,
            }
        }
    }
}

impl Source for FastAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        Some(self.current_samples.len())
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
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

    // Check if the sink has finished playing all queued audio
    pub async fn is_finished(&self) -> bool {
        let sink = self.sink.lock().await;
        sink.empty()
    }

    // Play from bytes at a specific position (supports MP3, AAC, and more via Symphonia!)
    pub async fn play_from_bytes_at_position(&self, data: Bytes, position: f32) -> Result<()> {
        info!("Fast seeking to {}s using Symphonia", position);

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

        // Create fast audio source on blocking thread
        let data_vec = data.to_vec();
        let source = tokio::task::spawn_blocking(move || FastAudioSource::new(data_vec, position))
            .await
            .context("Spawn task failed")?
            .context("Failed to create audio source")?;

        info!("Audio source ready, starting playback at {}s", position);

        // Replace the entire sink to start fresh
        let sink = self.sink.lock().await;

        sink.stop();
        drop(sink);

        // Create new sink
        let new_sink = Sink::try_new(&self._stream_handle).context("Failed to create new sink")?;
        new_sink.set_volume(volume);

        // Convert f32 samples to i16 for rodio compatibility
        let source_converted = source.convert_samples::<i16>();
        new_sink.append(source_converted);
        new_sink.play();

        let mut sink = self.sink.lock().await;
        *sink = new_sink;

        info!("Playing from {}s", position);
        Ok(())
    }
}
