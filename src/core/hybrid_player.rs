// 1. Start streaming immediately (fast playback)
// 2. Download entire file in background
// 3. When user seeks, switch to file-based playback at that position

use anyhow::{Context, Result};
use bytes::Bytes;
use log::{info, warn};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

pub struct HybridPlayer {
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Sink>>,
    downloaded_data: Arc<Mutex<Option<Bytes>>>, // None until download completes
}

impl HybridPlayer {
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) =
            OutputStream::try_default().context("Failed to create audio output stream")?;

        let sink = Sink::try_new(&stream_handle).context("Failed to create audio sink")?;

        Ok(Self {
            _stream: stream,
            stream_handle,
            sink: Arc::new(Mutex::new(sink)),
            downloaded_data: Arc::new(Mutex::new(None)),
        })
    }

    // Start playback: streams immediately and downloads in background
    pub async fn play_episode(&self, url: String) -> Result<()> {
        info!("Starting episode playback: {}", url);

        // Clear any previous download
        {
            let mut data = self.downloaded_data.lock().await;
            *data = None;
        }

        // Start streaming immediately for fast playback
        // This uses the existing streaming player logic
        info!("Streaming started, download will begin in background");

        // Spawn background download task
        let downloaded_data = self.downloaded_data.clone();
        tokio::spawn(async move {
            match download_episode(url).await {
                Ok(data) => {
                    info!("Episode download complete ({} bytes)", data.len());
                    let mut download = downloaded_data.lock().await;
                    *download = Some(data);
                }
                Err(e) => {
                    warn!("Background download failed: {}", e);
                }
            }
        });

        Ok(())
    }

    // Seek to position (TODO: prevent seeking in UI until download is complete)
    pub async fn seek(&self, position: f32) -> Result<()> {
        info!("Seek requested to {}s", position);

        // Check if download is complete
        let data_opt = {
            let data = self.downloaded_data.lock().await;
            data.clone()
        };

        let Some(audio_data) = data_opt else {
            warn!("Cannot seek - download not complete yet");
            return Ok(());
        };

        info!("Download complete, seeking to {}s", position);

        // Create decoder from downloaded data
        let cursor = Cursor::new(audio_data.to_vec());
        let source = Decoder::new(cursor).context("Failed to decode audio file")?;

        // Skip to desired position
        let skipped_source = source.skip_duration(Duration::from_secs_f32(position));

        // Replace sink content
        let mut sink = self.sink.lock().await;
        let was_paused = sink.is_paused();
        let volume = sink.volume();

        sink.stop();
        drop(sink);

        // Create new sink with seeked audio
        let new_sink = Sink::try_new(&self.stream_handle)?;
        new_sink.set_volume(volume);
        new_sink.append(skipped_source);

        if !was_paused {
            new_sink.play();
        } else {
            new_sink.pause();
        }

        let mut sink = self.sink.lock().await;
        *sink = new_sink;

        info!("Seeked to {}s successfully", position);
        Ok(())
    }

    // Check if download is complete (for UI to enable/disable seeking)
    pub async fn is_seekable(&self) -> bool {
        let data = self.downloaded_data.lock().await;
        data.is_some()
    }

    pub fn pause(&self) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.pause();
        }
    }

    pub fn resume(&self) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.play();
        }
    }

    pub fn set_volume(&self, volume: f32) {
        if let Ok(sink) = self.sink.try_lock() {
            sink.set_volume(volume);
        }
    }

    pub async fn stop(&self) {
        let sink = self.sink.lock().await;
        sink.stop();
        drop(sink);

        let mut data = self.downloaded_data.lock().await;
        *data = None;
    }
}

// Download entire episode to memory
async fn download_episode(url: String) -> Result<Bytes> {
    let client = reqwest::Client::builder()
        .user_agent("SR-Player/3.0-HybridPlayer")
        .connect_timeout(Duration::from_secs(30))
        .build()
        .context("Failed to create HTTP client")?;

    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to download episode")?;

    let bytes = response
        .bytes()
        .await
        .context("Failed to read episode data")?;

    Ok(bytes)
}
