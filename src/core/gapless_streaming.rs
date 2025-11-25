// Gapless audio streaming with Symphonia decoder and DVR buffer

use crate::core::dvr_buffer::DvrBuffer;
use crate::core::utils::audio_buffer_to_i16;
use anyhow::{Context, Result};
use bytes::Bytes;
use crossbeam_channel;
use log::{error, info, warn};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
#[allow(dead_code)]
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
            bitrate_kbps: 0.0, buffer_bytes: 0, chunks_queued: 0,
            download_speed_kbps: 0.0, bytes_downloaded: 0, last_update: Instant::now(),
        }
    }
}

// Channel-based MediaSource bridging async downloads to sync Symphonia decoder
struct ChannelMediaSource {
    receiver: crossbeam_channel::Receiver<Bytes>,
    current_chunk: Option<Bytes>,
    current_position: usize,
    is_eof: bool,
}

impl ChannelMediaSource {
    fn new(receiver: crossbeam_channel::Receiver<Bytes>) -> Self {
        Self { receiver, current_chunk: None, current_position: 0, is_eof: false }
    }
}

impl std::io::Read for ChannelMediaSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.is_eof { return Ok(0); }

        let mut total_read = 0;
        while total_read < buf.len() {
            if self.current_chunk.is_none() || self.current_position >= self.current_chunk.as_ref().unwrap().len() {
                match self.receiver.recv() {
                    Ok(chunk) => { self.current_chunk = Some(chunk); self.current_position = 0; }
                    Err(_) => { self.is_eof = true; break; }
                }
            }

            if let Some(ref chunk) = self.current_chunk {
                let to_copy = (chunk.len() - self.current_position).min(buf.len() - total_read);
                buf[total_read..total_read + to_copy]
                    .copy_from_slice(&chunk[self.current_position..self.current_position + to_copy]);
                self.current_position += to_copy;
                total_read += to_copy;
            }
        }
        Ok(total_read)
    }
}

impl std::io::Seek for ChannelMediaSource {
    fn seek(&mut self, _pos: std::io::SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Cannot seek in live stream"))
    }
}

impl MediaSource for ChannelMediaSource {
    fn is_seekable(&self) -> bool { false }
    fn byte_len(&self) -> Option<u64> { None }
}

// Custom rodio Source that pulls decoded samples from Symphonia
struct SymphoniaSource {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    current_buffer: Option<Vec<i16>>,
    buffer_position: usize,
    sample_rate: u32,
    channels: u16,
}

impl SymphoniaSource {
    fn new(media_source: Box<dyn MediaSource>) -> Result<Self> {
        let mss = MediaSourceStream::new(media_source, Default::default());
        let mut hint = Hint::new();
        hint.with_extension("aac");

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .context("Failed to probe audio format")?;

        let format_reader = probed.format;
        let track = format_reader.default_track().context("No audio tracks found")?;
        let track_id = track.id;
        let codec_params = &track.codec_params;
        let sample_rate = codec_params.sample_rate.context("No sample rate")?;
        let channels = codec_params.channels.context("No channels")?.count() as u16;

        info!("Detected audio: {}Hz, {} channels", sample_rate, channels);

        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .context("Failed to create decoder")?;

        Ok(Self {
            format_reader, decoder, track_id,
            current_buffer: None, buffer_position: 0, sample_rate, channels,
        })
    }

    fn decode_next_packet(&mut self) -> Result<bool> {
        let packet = match self.format_reader.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(e) => return Err(anyhow::anyhow!("Error reading packet: {}", e)),
        };

        if packet.track_id() != self.track_id { return Ok(true); }

        match self.decoder.decode(&packet) {
            Ok(decoded) => {
                self.current_buffer = Some(audio_buffer_to_i16(&decoded));
                self.buffer_position = 0;
                Ok(true)
            }
            Err(e) => { warn!("Decode error: {}", e); Ok(true) }
        }
    }
}

impl Iterator for SymphoniaSource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref buffer) = self.current_buffer {
                if self.buffer_position < buffer.len() {
                    let sample = buffer[self.buffer_position];
                    self.buffer_position += 1;
                    return Some(sample);
                }
            }
            match self.decode_next_packet() {
                Ok(true) => continue,
                Ok(false) => return None,
                Err(e) => { error!("Decode error: {}", e); return None; }
            }
        }
    }
}

impl Source for SymphoniaSource {
    fn current_frame_len(&self) -> Option<usize> { None }
    fn channels(&self) -> u16 { self.channels }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn total_duration(&self) -> Option<Duration> { None }
}

enum StreamCommand { Stop }

pub struct GaplessPlayer {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    command_tx: Option<mpsc::UnboundedSender<StreamCommand>>,
    download_handle: Option<JoinHandle<()>>,
    decoder_handle: Option<tokio::task::JoinHandle<()>>,
    stream_generation: Arc<AtomicU64>,
    current_generation: u64,
    http_client: reqwest::Client,
    sink: Arc<Mutex<Sink>>,
    stats: Arc<Mutex<StreamStats>>,
    live_buffer: Arc<DvrBuffer>,
    buffer_start_time: Arc<Mutex<Option<Instant>>>,
    is_at_live_edge: Arc<Mutex<bool>>,
}

impl GaplessPlayer {
    pub fn new_with_client(http_client: Option<reqwest::Client>) -> Result<Self> {
        let (stream, stream_handle) = OutputStream::try_default().context("Failed to create audio output stream")?;
        let sink = Sink::try_new(&stream_handle).context("Failed to create audio sink")?;

        let http_client = http_client.unwrap_or_else(|| {
            reqwest::Client::builder()
                .user_agent("SR-Player/3.0-Gapless")
                .pool_max_idle_per_host(5)
                .pool_idle_timeout(Duration::from_secs(90))
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(10))
                .tcp_keepalive(Some(Duration::from_secs(30)))
                .http2_keep_alive_interval(Some(Duration::from_secs(10)))
                .http2_keep_alive_timeout(Duration::from_secs(20))
                .build()
                .expect("Failed to create HTTP client")
        });

        Ok(Self {
            _stream: stream, _stream_handle: stream_handle,
            command_tx: None, download_handle: None, decoder_handle: None,
            stream_generation: Arc::new(AtomicU64::new(0)), current_generation: 0,
            http_client, sink: Arc::new(Mutex::new(sink)),
            stats: Arc::new(Mutex::new(StreamStats::new())),
            live_buffer: Arc::new(DvrBuffer::new()),
            buffer_start_time: Arc::new(Mutex::new(None)),
            is_at_live_edge: Arc::new(Mutex::new(true)),
        })
    }

    pub async fn start_stream(&mut self, url: String) -> Result<()> {
        info!("Starting gapless stream: {}", url);
        self.stop().await;

        self.current_generation += 1;
        self.stream_generation.store(self.current_generation, Ordering::SeqCst);
        let generation = self.current_generation;

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (data_tx, data_rx) = crossbeam_channel::unbounded::<Bytes>();
        self.command_tx = Some(command_tx);

        self.download_handle = Some(self.spawn_download_task(url, generation, command_rx, data_tx));
        self.decoder_handle = Some(self.spawn_decoder_task(generation, data_rx));

        Ok(())
    }

    fn spawn_download_task(
        &self, url: String, generation: u64,
        mut command_rx: mpsc::UnboundedReceiver<StreamCommand>,
        data_tx: crossbeam_channel::Sender<Bytes>,
    ) -> JoinHandle<()> {
        let gen_check = self.stream_generation.clone();
        let client = self.http_client.clone();
        let stats = self.stats.clone();
        let live_buffer = self.live_buffer.clone();
        let buffer_start_time = self.buffer_start_time.clone();

        tokio::spawn(async move {
            if gen_check.load(Ordering::SeqCst) != generation { return; }

            let Some((mut response, is_finite)) = Self::connect_with_retry(&client, &url).await else { return };

            // Buffer initial data
            let mut bytes_downloaded = 0;
            let mut initial_buffer = Vec::new();
            while initial_buffer.len() < 2048 {
                match response.chunk().await {
                    Ok(Some(chunk)) => { bytes_downloaded += chunk.len(); initial_buffer.extend_from_slice(&chunk); }
                    _ => {
                        let Some((new_resp, _)) = Self::connect_with_retry(&client, &url).await else { return };
                        response = new_resp;
                    }
                }
            }

            if gen_check.load(Ordering::SeqCst) != generation || data_tx.send(Bytes::from(initial_buffer)).is_err() { return; }

            let mut reconnect_count = 0;
            loop {
                if gen_check.load(Ordering::SeqCst) != generation { break; }

                let start_time = Instant::now();
                match Self::stream_chunks(
                    response, generation, &gen_check, &mut command_rx, &data_tx,
                    &stats, &live_buffer, &buffer_start_time, &mut bytes_downloaded, start_time, is_finite,
                ).await {
                    Ok(()) => break,
                    Err(()) => {
                        reconnect_count += 1;
                        if reconnect_count > 10 { break; }
                        if reconnect_count > 1 { tokio::time::sleep(Duration::from_millis(200 * (reconnect_count - 1) as u64)).await; }
                        let Some((new_resp, _)) = Self::connect_with_retry(&client, &url).await else { break };
                        response = new_resp;
                        reconnect_count = 0;
                    }
                }
            }
        })
    }

    async fn connect_with_retry(client: &reqwest::Client, url: &str) -> Option<(reqwest::Response, bool)> {
        for attempt in 1..=3 {
            match tokio::time::timeout(Duration::from_secs(5), client.get(url).send()).await {
                Ok(Ok(resp)) => {
                    let is_finite = resp.content_length().is_some();
                    return Some((resp, is_finite));
                }
                _ => {
                    if attempt < 3 { tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await; }
                }
            }
        }
        error!("Failed to connect after 3 retries");
        None
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_chunks(
        mut response: reqwest::Response, generation: u64, gen_check: &Arc<AtomicU64>,
        command_rx: &mut mpsc::UnboundedReceiver<StreamCommand>,
        data_tx: &crossbeam_channel::Sender<Bytes>, stats: &Arc<Mutex<StreamStats>>,
        live_buffer: &Arc<DvrBuffer>, buffer_start_time: &Arc<Mutex<Option<Instant>>>,
        bytes_downloaded: &mut usize, start_time: Instant, is_finite: bool,
    ) -> Result<(), ()> {
        loop {
            if gen_check.load(Ordering::SeqCst) != generation { return Ok(()); }
            if let Ok(StreamCommand::Stop) = command_rx.try_recv() { return Ok(()); }

            match tokio::time::timeout(Duration::from_secs(3), response.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    *bytes_downloaded += chunk.len();
                    if let Ok(mut s) = stats.try_lock() {
                        let elapsed = start_time.elapsed().as_secs_f32();
                        s.bytes_downloaded = *bytes_downloaded;
                        s.download_speed_kbps = (*bytes_downloaded as f32 * 8.0) / (elapsed * 1000.0);
                        s.bitrate_kbps = s.download_speed_kbps;
                    }
                    live_buffer.push(chunk.clone()).await;
                    if buffer_start_time.lock().await.is_none() {
                        *buffer_start_time.lock().await = Some(Instant::now());
                    }
                    if data_tx.send(chunk).is_err() { return Ok(()); }
                }
                Ok(Ok(None)) => return if is_finite { Ok(()) } else { Err(()) },
                Ok(Err(_)) | Err(_) => return Err(()),
            }
        }
    }

    fn spawn_decoder_task(&self, generation: u64, data_rx: crossbeam_channel::Receiver<Bytes>) -> tokio::task::JoinHandle<()> {
        let gen_check = self.stream_generation.clone();
        let sink = self.sink.clone();

        tokio::task::spawn_blocking(move || {
            if gen_check.load(Ordering::SeqCst) != generation { return; }

            let first_chunk = loop {
                match data_rx.recv_timeout(Duration::from_secs(10)) {
                    Ok(chunk) => break chunk,
                    Err(_) if gen_check.load(Ordering::SeqCst) != generation => return,
                    Err(_) => continue,
                }
            };

            let mut media_source = Box::new(ChannelMediaSource::new(data_rx));
            media_source.current_chunk = Some(first_chunk);

            let symphonia_source = match SymphoniaSource::new(media_source) {
                Ok(source) => source,
                Err(e) => { error!("Failed to create Symphonia source: {}", e); return; }
            };

            let sink_guard = tokio::runtime::Handle::current().block_on(sink.lock());
            sink_guard.append(symphonia_source);
            sink_guard.play();
            info!("Playback started!");
        })
    }

    pub async fn stop(&mut self) {
        if let Some(tx) = self.command_tx.take() { let _ = tx.send(StreamCommand::Stop); }
        if let Some(h) = self.download_handle.take() { h.abort(); }
        if let Some(h) = self.decoder_handle.take() { h.abort(); }
        self.sink.lock().await.stop();
        self.live_buffer.clear().await;
        *self.buffer_start_time.lock().await = None;
        *self.is_at_live_edge.lock().await = true;
    }

    pub fn get_stats(&self) -> StreamStats {
        self.stats.try_lock().map(|s| s.clone()).unwrap_or_else(|_| StreamStats::new())
    }

    #[allow(dead_code)]
    pub async fn is_playing(&self) -> bool {
        let sink = self.sink.lock().await;
        !sink.is_paused() && !sink.empty()
    }

    pub fn pause(&self) { if let Ok(sink) = self.sink.try_lock() { sink.pause(); } }
    pub fn resume(&self) { if let Ok(sink) = self.sink.try_lock() { sink.play(); } }
    pub fn set_volume(&self, volume: f32) { if let Ok(sink) = self.sink.try_lock() { sink.set_volume(volume); } }

    pub async fn get_buffer_depth(&self) -> Duration { self.live_buffer.get_buffer_depth().await }

    pub async fn get_buffer_time_range(&self) -> (f32, f32) {
        if let Some(start_time) = *self.buffer_start_time.lock().await {
            let elapsed = start_time.elapsed().as_secs_f32();
            let depth = self.live_buffer.get_buffer_depth().await.as_secs_f32();
            (elapsed - depth, elapsed)
        } else {
            (0.0, 0.0)
        }
    }

    pub async fn can_seek_to(&self, offset_secs: f32) -> bool {
        let (oldest, newest) = self.get_buffer_time_range().await;
        offset_secs >= oldest && offset_secs <= newest
    }

    pub async fn get_buffer_data_from_offset(&self, offset_secs: f32) -> Result<Vec<u8>> {
        if self.buffer_start_time.lock().await.is_some() {
            if let Some(chunks) = self.live_buffer.get_from_offset(Duration::from_secs_f32(offset_secs)).await {
                return Ok(chunks.into_iter().flat_map(|c| c.to_vec()).collect());
            }
        }
        anyhow::bail!("No data available at offset {}", offset_secs)
    }

    pub async fn seek_to(&mut self, offset_secs: f32) -> Result<f32> {
        if !self.can_seek_to(offset_secs).await {
            anyhow::bail!("Cannot seek to {}: out of buffer range", offset_secs);
        }
        Ok(offset_secs)
    }

    pub async fn is_at_live_edge(&self) -> bool { *self.is_at_live_edge.lock().await }
    pub async fn set_at_live_edge(&self, at_live: bool) { *self.is_at_live_edge.lock().await = at_live; }
}
