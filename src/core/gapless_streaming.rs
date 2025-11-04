// MILESTONE 3: Gapless audio streaming with Symphonia
//
// 1. HTTP Streaming Task: Downloads AAC data continuously from HTTP stream
// 2. Custom MediaSource: Implements std::io::Read over an async channel (mpsc)
// 3. Symphonia Decoder: Continuously decodes AAC frames from the MediaSource
// 4. Custom Rodio Source: Feeds decoded PCM samples directly to rodio's Sink
// 5. Rodio Sink: Plays audio without gaps
//
// Key difference from M2:
// - M2: Downloaded chunks -> Cursor -> rodio::Decoder -> Sink (GAPS between chunks)
// - M3: Stream -> MediaSource -> Symphonia -> Custom Source -> Sink (NO GAPS!)
//
// Compare to other languages:
// - JavaScript: Like using MediaSource API with SourceBuffer for gapless playback
// - Python: Like using PyAudio with continuous callback feeding PCM samples
//
// Rust concepts:
// - Custom Source trait impl: Similar to implementing an Iterator
// - std::io::Read over channel: Bridge between async downloads and sync decoding
// - Symphonia codecs: Direct access to low-level audio decoding

use anyhow::{Context, Result};
use bytes::Bytes;
use log::{info, warn, error};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex}; // tokio::sync::Mutex is Send + Sync friendly!
use tokio::task::JoinHandle;
use crossbeam_channel;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

// ============================================================================
// STREAMING STATISTICS
// ============================================================================

// Real-time statistics about the audio stream
#[derive(Debug, Clone)]
pub struct StreamStats {
    pub bitrate_kbps: f32,
    pub buffer_bytes: usize,
    pub chunks_queued: usize,
    pub download_speed_kbps: f32,
    pub bytes_downloaded: usize,
    pub last_update: Instant,
}

impl StreamStats {
    pub fn new() -> Self {
        Self {
            bitrate_kbps: 0.0,
            buffer_bytes: 0,
            chunks_queued: 0,
            download_speed_kbps: 0.0,
            bytes_downloaded: 0,
            last_update: Instant::now(),
        }
    }
}

// ============================================================================
// AUDIO SAMPLE PROCESSING HELPERS
// ============================================================================

// Generic helper to interleave audio buffer channels with sample conversion
//
// This eliminates code duplication between S16 and F32 audio buffer handling.
// The conversion function allows converting from any sample type to i16.
//
// # Arguments
// * `buf` - The audio buffer from AudioBufferRef (Cow type)
// * `convert` - Function to convert sample type T to i16
//
// # Returns
// Vec<i16> with samples interleaved: [L R L R L R...] for stereo, or [M M M...] for mono
fn interleave_audio_buffer<T>(
    buf: std::borrow::Cow<symphonia::core::audio::AudioBuffer<T>>,
    convert: impl Fn(T) -> i16,
) -> Vec<i16>
where
    T: symphonia::core::sample::Sample + Copy,
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

// ============================================================================
// CHANNEL-BASED MEDIA SOURCE
// ============================================================================

// Custom MediaSource that reads audio data from a crossbeam channel
//
// This bridges the async world with the sync world (std::io::Read) that Symphonia expects.
// It's like a pipe where one end receives bytes asynchronously and the other end can be read synchronously.
//
// We use crossbeam_channel because it's Sync + Send and works across threads
//
// In Python: Similar to queue.Queue with blocking get()
// In JavaScript: Like a ReadableStream that can be read synchronously
struct ChannelMediaSource {
    // Receiver end of the channel
    // crossbeam_channel::Receiver is both Sync and Send!
    receiver: crossbeam_channel::Receiver<Bytes>,

    // Current chunk being read from
    // When we receive a chunk, we store it here and read from it byte-by-byte
    current_chunk: Option<Bytes>,

    // Position within the current chunk
    current_position: usize,

    // Track if the stream has ended (channel closed)
    is_eof: bool,
}

impl ChannelMediaSource {
    fn new(receiver: crossbeam_channel::Receiver<Bytes>) -> Self {
        Self {
            receiver,
            current_chunk: None,
            current_position: 0,
            is_eof: false,
        }
    }
}

// Implement std::io::Read for our custom MediaSource
//
// This tells Rust: "You can read bytes from this thing just like reading from a file"
// Symphonia uses this to pull audio data as it needs it for decoding.
impl std::io::Read for ChannelMediaSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // If we've reached EOF, return 0 (standard way to signal end-of-stream)
        if self.is_eof {
            return Ok(0);
        }

        let mut total_read = 0;

        // Keep filling the buffer until it's full or we run out of data
        while total_read < buf.len() {
            // If we don't have a current chunk or we've exhausted it, get a new one
            if self.current_chunk.is_none() || self.current_position >= self.current_chunk.as_ref().unwrap().len() {
                match self.receiver.recv() {
                    Ok(chunk) => {
                        // Got new chunk!
                        self.current_chunk = Some(chunk);
                        self.current_position = 0;
                    }
                    Err(_) => {
                        // Channel closed, no more data coming
                        self.is_eof = true;
                        break;
                    }
                }
            }

            // Copy bytes from current chunk to the output buffer
            if let Some(ref chunk) = self.current_chunk {
                let remaining_in_chunk = chunk.len() - self.current_position;
                let remaining_in_buf = buf.len() - total_read;
                let to_copy = remaining_in_chunk.min(remaining_in_buf);

                buf[total_read..total_read + to_copy]
                    .copy_from_slice(&chunk[self.current_position..self.current_position + to_copy]);

                self.current_position += to_copy;
                total_read += to_copy;
            }
        }

        Ok(total_read)
    }
}

// Implement std::io::Seek for our custom MediaSource
//
// Even though we can't actually seek in a live stream, Symphonia requires
// this trait. We implement it as a no-op that always returns errors.
impl std::io::Seek for ChannelMediaSource {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        // Live streams can't seek!
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Cannot seek in live stream"
        ))
    }
}

// Implement MediaSource trait for Symphonia
//
// This tells Symphonia: "You can use this as a source of audio data"
// The main requirement is that it implements Read and Seek (which we did above)
impl MediaSource for ChannelMediaSource {
    fn is_seekable(&self) -> bool {
        // Live streams can't seek backwards!
        false
    }

    fn byte_len(&self) -> Option<u64> {
        // Live streams have unknown length
        None
    }
}

// ============================================================================
// CUSTOM RODIO SOURCE
// ============================================================================

// Custom rodio Source that pulls decoded audio samples from Symphonia
//
// - Symphonia decodes AAC frames into PCM samples
// - This Source feeds those samples to rodio's Sink
// - Rodio plays them through the speakers
//
// In Python: Like a generator that yields audio samples
// In JavaScript: Like an async iterator for audio data
struct SymphoniaSource {
    // The Symphonia format reader (handles container format like ADTS)
    format_reader: Box<dyn FormatReader>,

    // The audio decoder (handles AAC decoding)
    decoder: Box<dyn Decoder>,

    // Track ID we're decoding (usually track 0 for single-track streams)
    track_id: u32,

    // Current audio buffer with decoded samples
    // We keep this around to serve samples one at a time
    current_buffer: Option<Vec<i16>>,

    // Position within current buffer
    buffer_position: usize,

    // Audio sample rate (e.g., 44100 Hz)
    sample_rate: u32,

    // Number of channels (1 = mono, 2 = stereo)
    channels: u16,
}

impl SymphoniaSource {
    // Create a new SymphoniaSource from a MediaSource
    fn new(media_source: Box<dyn MediaSource>) -> Result<Self> {
        // Wrap the media source in a MediaSourceStream
        let mss = MediaSourceStream::new(media_source, Default::default());

        // Create a probe hint (tells Symphonia what format to expect)
        let mut hint = Hint::new();
        hint.with_extension("aac"); // We're expecting AAC

        // Probe the stream to detect format
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .context("Failed to probe audio format")?;

        let format_reader = probed.format;

        // Get the default track (usually track 0)
        let track = format_reader
            .default_track()
            .context("No audio tracks found")?;

        let track_id = track.id;

        // Get audio parameters
        let codec_params = &track.codec_params;
        let sample_rate = codec_params.sample_rate.context("No sample rate")?;
        let channels = codec_params.channels.context("No channels")?.count() as u16;

        info!("Detected audio: {}Hz, {} channels", sample_rate, channels);

        // Create decoder for this track
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .context("Failed to create decoder")?;

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            current_buffer: None,
            buffer_position: 0,
            sample_rate,
            channels,
        })
    }

    // Decode the next packet and store samples in current_buffer
    fn decode_next_packet(&mut self) -> Result<bool> {
        // Get next packet from format reader
        let packet = match self.format_reader.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // End of stream
                return Ok(false);
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Error reading packet: {}", e));
            }
        };

        // Skip packets from other tracks (shouldn't happen in single-track streams)
        if packet.track_id() != self.track_id {
            return Ok(true);
        }

        // Decode the packet
        let decoded = match self.decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(e) => {
                warn!("Decode error: {}", e);
                return Ok(true); // Skip this packet, try next one
            }
        };

        // Convert decoded audio to i16 samples
        // Symphonia can return different sample formats, we need i16 for rodio
        // IMPORTANT: Must interleave channels for stereo audio!
        let samples = match decoded {
            AudioBufferRef::S16(buf) => {
                // Already i16, just interleave channels
                interleave_audio_buffer(buf, |sample| sample)
            }
            AudioBufferRef::F32(buf) => {
                // Convert f32 to i16 and interleave channels
                interleave_audio_buffer(buf, |sample| (sample * 32767.0) as i16)
            }
            _ => {
                warn!("Unsupported audio format");
                return Ok(true);
            }
        };

        self.current_buffer = Some(samples);
        self.buffer_position = 0;

        Ok(true)
    }
}

// Implement rodio's Source trait for our custom source
//
// This tells rodio: "You can play audio from this thing"
// The Source trait requires:
// - current_frame_len(): How many samples available right now
// - channels(): Mono or stereo
// - sample_rate(): Sample rate in Hz
// - total_duration(): Total length (None for streams)
// - Iterator: Yields samples one by one
impl Iterator for SymphoniaSource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If we have samples in current buffer, return next sample
            if let Some(ref buffer) = self.current_buffer {
                if self.buffer_position < buffer.len() {
                    let sample = buffer[self.buffer_position];
                    self.buffer_position += 1;
                    return Some(sample);
                }
            }

            // Need to decode next packet
            match self.decode_next_packet() {
                Ok(true) => continue,  // Got new packet, try again
                Ok(false) => return None,  // End of stream
                Err(e) => {
                    error!("Decode error: {}", e);
                    return None;
                }
            }
        }
    }
}

impl Source for SymphoniaSource {
    fn current_frame_len(&self) -> Option<usize> {
        None // Unknown for streams
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None // Infinite stream
    }
}

// ============================================================================
// GAPLESS STREAMING PLAYER
// ============================================================================

// Commands to control the streaming player
enum StreamCommand {
    Stop,
}

// Gapless streaming audio player using Symphonia
pub struct GaplessPlayer {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    command_tx: Option<mpsc::UnboundedSender<StreamCommand>>,
    download_handle: Option<JoinHandle<()>>,
    sink: Arc<Mutex<Sink>>,
    stats: Arc<Mutex<StreamStats>>,
}

impl GaplessPlayer {
    // Create a new gapless streaming player
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default()
            .context("Failed to create audio output stream")?;

        let sink = Sink::try_new(&stream_handle)
            .context("Failed to create audio sink")?;

        Ok(Self {
            _stream: stream,
            _stream_handle: stream_handle,
            command_tx: None,
            download_handle: None,
            sink: Arc::new(Mutex::new(sink)),
            stats: Arc::new(Mutex::new(StreamStats::new())),
        })
    }

    // Start streaming from a URL with gapless playback
    pub async fn start_stream(&mut self, url: String) -> Result<()> {
        info!("Starting gapless stream: {}", url);

        // Stop any existing stream
        self.stop().await;

        // Create channels for communication
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();
        let (data_tx, data_rx) = crossbeam_channel::unbounded::<Bytes>();

        self.command_tx = Some(command_tx);

        let stats = self.stats.clone();
        let sink = self.sink.clone();

        // Spawn download task
        let download_handle = tokio::spawn(async move {
            info!("Download task started");

            // Create HTTP client with streaming support
            let client = match reqwest::Client::builder()
                .user_agent("SR-Player/3.0-Gapless")
                .connect_timeout(Duration::from_secs(30))
                .tcp_keepalive(Some(Duration::from_secs(30)))
                .build()
            {
                Ok(client) => client,
                Err(e) => {
                    error!("Failed to create HTTP client: {}", e);
                    return;
                }
            };

            // Start streaming request
            let mut response = match client.get(&url).send().await {
                Ok(resp) => resp,
                Err(e) => {
                    error!("Failed to connect: {}", e);
                    return;
                }
            };

            info!("Connected to stream, starting download...");

            let mut bytes_downloaded = 0;
            let start_time = Instant::now();

            // Stream bytes continuously
            while let Some(chunk_result) = response.chunk().await.transpose() {
                // Check for stop command (non-blocking)
                if let Ok(StreamCommand::Stop) = command_rx.try_recv() {
                    info!("Stop command received");
                    break;
                }

                match chunk_result {
                    Ok(chunk) => {
                        bytes_downloaded += chunk.len();

                        // Update stats
                        if let Ok(mut stats_guard) = stats.try_lock() {
                            let elapsed = start_time.elapsed().as_secs_f32();
                            stats_guard.bytes_downloaded = bytes_downloaded;
                            stats_guard.download_speed_kbps = (bytes_downloaded as f32 * 8.0) / (elapsed * 1000.0);
                            stats_guard.bitrate_kbps = stats_guard.download_speed_kbps;
                            stats_guard.last_update = Instant::now();
                        }

                        // Send chunk to decoder
                        if data_tx.send(chunk).is_err() {
                            // Receiver dropped, stop downloading
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Error reading chunk: {}", e);
                        break;
                    }
                }
            }

            info!("Download task finished");
        });

        self.download_handle = Some(download_handle);

        // Spawn decoder task
        let sink_clone = sink.clone();
        tokio::task::spawn_blocking(move || {
            info!("Decoder task started");

            // Create our custom media source from the channel
            let media_source = Box::new(ChannelMediaSource::new(data_rx));

            // Create Symphonia source
            let symphonia_source = match SymphoniaSource::new(media_source) {
                Ok(source) => source,
                Err(e) => {
                    error!("Failed to create Symphonia source: {}", e);
                    return;
                }
            };

            // Append to sink and play!
            let sink_guard = tokio::runtime::Handle::current().block_on(sink_clone.lock());
            sink_guard.append(symphonia_source);
            sink_guard.play();

            info!("Playback started!");
        });

        Ok(())
    }

    // Stop the current stream
    pub async fn stop(&mut self) {
        if let Some(tx) = self.command_tx.take() {
            let _ = tx.send(StreamCommand::Stop);
        }

        if let Some(handle) = self.download_handle.take() {
            handle.abort();
        }

        let sink = self.sink.lock().await;
        sink.stop();
    }

    // Get current streaming statistics
    pub fn get_stats(&self) -> StreamStats {
        if let Ok(stats) = self.stats.try_lock() {
            stats.clone()
        } else {
            StreamStats::new()
        }
    }

    // Check if audio is playing
    pub async fn is_playing(&self) -> bool {
        let sink = self.sink.lock().await;
        !sink.is_paused() && !sink.empty()
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

    // Set playback volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.set_volume(volume);
        }
    }
}
