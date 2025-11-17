// DVR Buffer System - Time-shifted playback for live radio
//
// This module implements a ring buffer that stores the most recent audio chunks
// from a live stream, allowing users to rewind and replay recent content.
//
// Architecture:
// 1. Ring Buffer: Fixed-size circular buffer storing audio chunks with timestamps
// 2. Continuous Write: New chunks push out old chunks automatically
// 3. Time-based Seeking: Seek by timestamp rather than byte position
// 4. Memory Management: Configurable buffer duration (default 30 minutes)
//
// Compare to:
// - DVR/TiVo: Records live TV for time-shifting
// - YouTube Live: Allows rewinding during live streams
// - Twitch VOD: Instant replay of live content

use bytes::Bytes;
use log::info;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// Default buffer duration: 30 minutes of audio
// At 128kbps (typical radio bitrate): ~28 MB
const DEFAULT_BUFFER_DURATION_SECS: u64 = 30 * 60;

// Estimated average bitrate for memory calculations (bytes per second)
// 128kbps = 16 KB/s
const ESTIMATED_BITRATE_BYTES_PER_SEC: usize = 16 * 1024;

// A single chunk of audio data with timing information
#[derive(Clone, Debug)]
struct AudioChunk {
    data: Bytes,
    timestamp: Instant,           // When this chunk was received
    offset_from_start: Duration,  // Time offset from buffer start
}

// DVR Buffer for time-shifted playback of live streams
//
// The buffer maintains a sliding window of recent audio chunks, allowing
// users to seek backwards in time while the live stream continues.
pub struct DvrBuffer {
    // Ring buffer of audio chunks (FIFO - oldest chunks get removed)
    chunks: Arc<RwLock<VecDeque<AudioChunk>>>,

    // Maximum buffer duration (how far back users can rewind)
    max_duration: Duration,

    // Maximum buffer size in bytes (prevents unbounded memory growth)
    max_size_bytes: usize,

    // Timestamp when the buffer started (first chunk received)
    start_time: Arc<RwLock<Option<Instant>>>,

    // Statistics
    total_bytes: Arc<RwLock<usize>>,
    total_chunks: Arc<RwLock<usize>>,
}

impl DvrBuffer {
    // Create a new DVR buffer with default 30-minute capacity
    pub fn new() -> Self {
        Self::with_duration(Duration::from_secs(DEFAULT_BUFFER_DURATION_SECS))
    }

    // Create a DVR buffer with custom duration
    pub fn with_duration(duration: Duration) -> Self {
        let max_size_bytes = duration.as_secs() as usize * ESTIMATED_BITRATE_BYTES_PER_SEC;

        info!(
            "Creating DVR buffer: duration={}s, max_size={}MB",
            duration.as_secs(),
            max_size_bytes / (1024 * 1024)
        );

        Self {
            chunks: Arc::new(RwLock::new(VecDeque::new())),
            max_duration: duration,
            max_size_bytes,
            start_time: Arc::new(RwLock::new(None)),
            total_bytes: Arc::new(RwLock::new(0)),
            total_chunks: Arc::new(RwLock::new(0)),
        }
    }

    // Add a new audio chunk to the buffer
    //
    // This appends the chunk to the end and removes old chunks if necessary
    // to maintain the maximum duration/size constraints.
    pub async fn push(&self, data: Bytes) {
        let chunk_size = data.len();
        let now = Instant::now();

        let mut chunks = self.chunks.write().await;
        let mut start_time = self.start_time.write().await;
        let mut total_bytes = self.total_bytes.write().await;
        let mut total_chunks = self.total_chunks.write().await;

        // Initialize start time on first chunk
        if start_time.is_none() {
            *start_time = Some(now);
        }

        let start = start_time.unwrap();
        let offset_from_start = now.duration_since(start);

        // Create and add the new chunk
        let chunk = AudioChunk {
            data,
            timestamp: now,
            offset_from_start,
        };

        chunks.push_back(chunk);
        *total_bytes += chunk_size;
        *total_chunks += 1;

        // Remove old chunks that exceed our duration or size limits
        while !chunks.is_empty() {
            let should_remove = if let Some(oldest) = chunks.front() {
                let age = now.duration_since(oldest.timestamp);
                age > self.max_duration || *total_bytes > self.max_size_bytes
            } else {
                false
            };

            if should_remove {
                if let Some(removed) = chunks.pop_front() {
                    *total_bytes = total_bytes.saturating_sub(removed.data.len());
                }
            } else {
                break;
            }
        }
    }

    // Get buffered audio data from a specific time offset
    //
    // Returns all chunks starting from the given offset until the end of the buffer.
    // This allows seeking to any point in the buffered history.
    //
    // # Arguments
    // * `offset` - Time offset from the buffer start (0 = earliest buffered content)
    //
    // # Returns
    // Vector of audio chunks from the offset onwards, or None if offset is invalid
    pub async fn get_from_offset(&self, offset: Duration) -> Option<Vec<Bytes>> {
        let chunks = self.chunks.read().await;

        if chunks.is_empty() {
            return None;
        }

        // Find the first chunk at or after the requested offset
        let mut result = Vec::new();

        for chunk in chunks.iter() {
            if chunk.offset_from_start >= offset {
                result.push(chunk.data.clone());
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    // Get the current buffer depth (how far back we can rewind)
    //
    // Returns the duration between the oldest and newest chunks in the buffer.
    pub async fn get_buffer_depth(&self) -> Duration {
        let chunks = self.chunks.read().await;

        if chunks.is_empty() {
            return Duration::from_secs(0);
        }

        let oldest = chunks.front().unwrap();
        let newest = chunks.back().unwrap();

        newest.timestamp.duration_since(oldest.timestamp)
    }

    // Get all buffered audio data as a single continuous stream
    //
    // This concatenates all chunks into a single byte vector.
    // Useful for switching to file-based playback.
    #[allow(dead_code)]
    pub async fn get_all_data(&self) -> Vec<u8> {
        let chunks = self.chunks.read().await;

        let total_size: usize = chunks.iter().map(|c| c.data.len()).sum();
        let mut result = Vec::with_capacity(total_size);

        for chunk in chunks.iter() {
            result.extend_from_slice(&chunk.data);
        }

        result
    }

    // Clear the buffer (used when stopping or switching channels)
    pub async fn clear(&self) {
        let mut chunks = self.chunks.write().await;
        let mut start_time = self.start_time.write().await;
        let mut total_bytes = self.total_bytes.write().await;
        let mut total_chunks = self.total_chunks.write().await;

        chunks.clear();
        *start_time = None;
        *total_bytes = 0;
        *total_chunks = 0;

        info!("DVR buffer cleared");
    }

    // Get buffer statistics
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> DvrBufferStats {
        let total_bytes = *self.total_bytes.read().await;
        let total_chunks = *self.total_chunks.read().await;
        let buffer_depth = self.get_buffer_depth().await;

        DvrBufferStats {
            total_bytes,
            total_chunks,
            buffer_depth_secs: buffer_depth.as_secs_f32(),
            max_duration_secs: self.max_duration.as_secs_f32(),
        }
    }
}

// Statistics about the DVR buffer
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DvrBufferStats {
    pub total_bytes: usize,
    pub total_chunks: usize,
    pub buffer_depth_secs: f32,
    pub max_duration_secs: f32,
}

impl std::fmt::Display for DvrBufferStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DVR Buffer: {:.1}MB, {} chunks, {:.1}s / {:.1}s",
            self.total_bytes as f32 / (1024.0 * 1024.0),
            self.total_chunks,
            self.buffer_depth_secs,
            self.max_duration_secs
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_dvr_buffer_basic() {
        let buffer = DvrBuffer::new();

        // Add some chunks
        buffer.push(Bytes::from("chunk1")).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        buffer.push(Bytes::from("chunk2")).await;

        // Check stats
        let stats = buffer.get_stats().await;
        assert_eq!(stats.total_chunks, 2);
        assert!(stats.buffer_depth_secs > 0.0);
    }

    #[tokio::test]
    async fn test_dvr_buffer_overflow() {
        // Create a small buffer (1 second, ~16KB)
        let buffer = DvrBuffer::with_duration(Duration::from_secs(1));

        // Add chunks that will overflow the buffer
        for i in 0..100 {
            let data = vec![0u8; 1024]; // 1KB per chunk
            buffer.push(Bytes::from(data)).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Buffer should have removed old chunks
        let stats = buffer.get_stats().await;
        assert!(stats.total_chunks < 100);
    }
}
