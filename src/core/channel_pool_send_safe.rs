// Send-safe wrapper for ChannelPool
//
// This wrapper allows the channel pool to be safely used from Slint's UI thread.
// Similar to SendSafeGaplessPlayer, this provides a thread-safe interface.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::channel_pool::ChannelPool;

/// Thread-safe wrapper for ChannelPool that can be used from Slint's UI thread
#[derive(Clone)]
pub struct SendSafeChannelPool {
    pool: Arc<Mutex<ChannelPool>>,
}

impl SendSafeChannelPool {
    /// Create a new send-safe channel pool
    pub async fn new() -> Result<Self> {
        let pool = ChannelPool::new()?;

        Ok(Self {
            pool: Arc::new(Mutex::new(pool)),
        })
    }

    /// Switch to a channel (instant if already in pool, otherwise streams it)
    pub async fn switch_to_channel(&self, channel_id: i32, url: String) -> Result<()> {
        let pool = self.pool.lock().await;
        pool.switch_to_channel(channel_id, url).await
    }

    /// Stop all streams and clear the pool
    pub async fn stop_all(&self) {
        let pool = self.pool.lock().await;
        pool.stop_all().await;
    }

    /// Set volume for the currently playing channel
    pub async fn set_volume(&self, volume: f32) {
        let pool = self.pool.lock().await;
        pool.set_volume(volume).await;
    }

    /// Pause the currently playing channel
    pub async fn pause(&self) {
        let pool = self.pool.lock().await;
        pool.pause().await;
    }

    /// Resume the currently playing channel
    pub async fn resume(&self) {
        let pool = self.pool.lock().await;
        pool.resume().await;
    }

    /// Get DVR buffer time range for the currently playing channel
    pub async fn get_buffer_time_range(&self) -> (f32, f32) {
        let pool = self.pool.lock().await;
        pool.get_buffer_time_range().await
    }

    /// Get buffered audio data from a specific offset for the currently playing channel
    pub async fn get_buffer_data_from_offset(&self, offset_secs: f32) -> Result<Vec<u8>> {
        let pool = self.pool.lock().await;
        pool.get_buffer_data_from_offset(offset_secs).await
    }

    /// Set whether the current channel is at live edge
    pub async fn set_at_live_edge(&self, at_live: bool) {
        let pool = self.pool.lock().await;
        pool.set_at_live_edge(at_live).await;
    }
}
