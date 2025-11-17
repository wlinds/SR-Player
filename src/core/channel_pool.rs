// Multi-channel background streaming pool
//
// This module manages multiple audio streams simultaneously:
// - Keeps up to 3 channels streaming in the background
// - Allows instant switching between pre-loaded channels
// - Automatically manages the pool (evicts least recently used channels)
//
// Architecture:
// - Each channel has its own GaplessPlayer instance
// - Only one channel plays audio at a time (others are muted via Sink::set_volume(0.0))
// - When switching channels, we just adjust volumes (instant!)
// - New channels are added to the pool, oldest are evicted when pool is full

use anyhow::Result;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use super::gapless_send_safe::SendSafeGaplessPlayer;

const MAX_BACKGROUND_CHANNELS: usize = 3;

struct PooledChannel {
    player: SendSafeGaplessPlayer,
    last_used: std::time::Instant,
}

pub struct ChannelPool {
    // Map of channel_id -> PooledChannel
    channels: Arc<Mutex<HashMap<i32, PooledChannel>>>,
    // Currently playing channel ID
    current_channel: Arc<Mutex<Option<i32>>>,
    // Shared HTTP client for connection pooling across all players
    http_client: reqwest::Client,
}

impl ChannelPool {
    pub fn new() -> Result<Self> {
        // Create shared HTTP client with connection pooling
        // This client is shared across all GaplessPlayer instances!
        // Connection pooling means: when we switch channels, we reuse existing TCP connections
        let http_client = reqwest::Client::builder()
            .user_agent("SR-Player/3.0-Gapless")
            .pool_max_idle_per_host(10)         // Keep up to 10 idle connections per host
            .pool_idle_timeout(Duration::from_secs(120))  // Keep connections alive for 2 minutes
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .http2_keep_alive_interval(Some(Duration::from_secs(10)))
            .http2_keep_alive_timeout(Duration::from_secs(20))
            .build()?;

        Ok(Self {
            channels: Arc::new(Mutex::new(HashMap::new())),
            current_channel: Arc::new(Mutex::new(None)),
            http_client,
        })
    }

    /// Switch to a channel (starts streaming if not already in pool)
    pub async fn switch_to_channel(&self, channel_id: i32, url: String) -> Result<()> {
        let mut channels = self.channels.lock().await;
        let mut current = self.current_channel.lock().await;

        // Mute currently playing channel
        if let Some(current_id) = *current {
            if let Some(channel) = channels.get(&current_id) {
                channel.player.set_volume(0.0);
                info!("Muted channel {}", current_id);
            }
        }

        // Check if channel is already in the pool
        if let Some(channel) = channels.get_mut(&channel_id) {
            // Channel already streaming! Just unmute it
            info!("Channel {} already in pool, instant switch!", channel_id);
            channel.player.set_volume(1.0);
            channel.last_used = std::time::Instant::now();
            *current = Some(channel_id);
            return Ok(());
        }

        // Channel not in pool - need to start streaming
        info!("Channel {} not in pool, starting stream...", channel_id);

        // Evict oldest channel if pool is full
        if channels.len() >= MAX_BACKGROUND_CHANNELS {
            // Find the least recently used channel (that's not the current one)
            let lru_id = channels
                .iter()
                .filter(|(id, _)| Some(**id) != *current)
                .min_by_key(|(_, ch)| ch.last_used)
                .map(|(id, _)| *id);

            if let Some(lru_id) = lru_id {
                info!("Pool full, evicting channel {}", lru_id);
                if let Some(old_channel) = channels.remove(&lru_id) {
                    old_channel.player.stop().await;
                }
            }
        }

        // Create new player with shared HTTP client for connection pooling
        let player = SendSafeGaplessPlayer::new_with_client(Some(self.http_client.clone()))?;
        player.start_stream(url.clone()).await?;
        player.set_volume(1.0); // This will be the active channel

        channels.insert(
            channel_id,
            PooledChannel {
                player,
                last_used: std::time::Instant::now(),
            },
        );

        *current = Some(channel_id);
        info!("Channel {} added to pool and playing", channel_id);

        Ok(())
    }

    /// Stop all streams and clear the pool
    pub async fn stop_all(&self) {
        let mut channels = self.channels.lock().await;
        for (_, channel) in channels.drain() {
            channel.player.stop().await;
        }
        *self.current_channel.lock().await = None;
        info!("Stopped all channels and cleared pool");
    }

    /// Set volume for the currently playing channel
    pub async fn set_volume(&self, volume: f32) {
        let current = self.current_channel.lock().await;
        if let Some(current_id) = *current {
            let channels = self.channels.lock().await;
            if let Some(channel) = channels.get(&current_id) {
                channel.player.set_volume(volume);
            }
        }
    }

    /// Pause the currently playing channel
    pub async fn pause(&self) {
        let current = self.current_channel.lock().await;
        if let Some(current_id) = *current {
            let channels = self.channels.lock().await;
            if let Some(channel) = channels.get(&current_id) {
                channel.player.pause();
            }
        }
    }

    /// Resume the currently playing channel
    pub async fn resume(&self) {
        let current = self.current_channel.lock().await;
        if let Some(current_id) = *current {
            let channels = self.channels.lock().await;
            if let Some(channel) = channels.get(&current_id) {
                channel.player.resume();
            }
        }
    }

    /// Get DVR buffer time range for the currently playing channel
    pub async fn get_buffer_time_range(&self) -> (f32, f32) {
        let current = self.current_channel.lock().await;
        if let Some(current_id) = *current {
            let channels = self.channels.lock().await;
            if let Some(channel) = channels.get(&current_id) {
                return channel.player.get_buffer_time_range().await;
            }
        }
        (0.0, 0.0)
    }

    /// Get buffered audio data from a specific offset for the currently playing channel
    pub async fn get_buffer_data_from_offset(&self, offset_secs: f32) -> Result<Vec<u8>> {
        let current = self.current_channel.lock().await;
        if let Some(current_id) = *current {
            let channels = self.channels.lock().await;
            if let Some(channel) = channels.get(&current_id) {
                return channel.player.get_buffer_data_from_offset(offset_secs).await;
            }
        }
        anyhow::bail!("No active channel")
    }

    /// Set whether the current channel is at live edge
    pub async fn set_at_live_edge(&self, at_live: bool) {
        let current = self.current_channel.lock().await;
        if let Some(current_id) = *current {
            let channels = self.channels.lock().await;
            if let Some(channel) = channels.get(&current_id) {
                channel.player.set_at_live_edge(at_live).await;
            }
        }
    }
}
