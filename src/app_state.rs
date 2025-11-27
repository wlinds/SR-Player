// Application State - Bundles all shared state for cleaner callback setup
//
// This struct holds all the thread-safe references to players, caches, and state
// that are shared across UI callbacks. By bundling them together, we reduce
// the number of individual .clone() calls needed in each callback setup.

use crate::core::channel_pool::ChannelPool;
use crate::core::episode_cache::EpisodeCache;
use crate::core::favorites::Favorites;
use crate::core::file_player_send_safe::SendSafeFilePlayer;
use crate::core::gapless_send_safe::SendSafeGaplessPlayer;
use crate::core::models::{ActivePlayer, Program};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

/// Bundles all shared application state for use in UI callbacks
#[derive(Clone)]
pub struct AppState {
    // Audio players
    pub channel_pool: ChannelPool,
    pub streaming_player: Arc<SendSafeGaplessPlayer>,
    pub file_player: Arc<SendSafeFilePlayer>,

    // Caching
    pub episode_cache: EpisodeCache,

    // Playback state
    pub active_player: Arc<Mutex<ActivePlayer>>,
    pub current_episode_url: Arc<Mutex<Option<String>>>,
    pub current_channel_id: Arc<Mutex<Option<i32>>>,
    pub current_episode_index: Arc<Mutex<Option<usize>>>,
    pub current_episodes: Arc<Mutex<Vec<crate::EpisodeItem>>>,

    // Program state
    pub all_programs: Arc<Mutex<Vec<Program>>>,
    pub groups_expanded: Arc<Mutex<HashMap<i32, bool>>>,

    // Favorites
    pub favorites: Arc<Mutex<Favorites>>,

    // Runtime handle for spawning async tasks
    pub runtime: Handle,
}

impl AppState {
    /// Create a new AppState with all components initialized
    pub fn new(
        channel_pool: ChannelPool,
        streaming_player: Arc<SendSafeGaplessPlayer>,
        file_player: Arc<SendSafeFilePlayer>,
        episode_cache: EpisodeCache,
        favorites: Favorites,
        runtime: Handle,
    ) -> Self {
        Self {
            channel_pool,
            streaming_player,
            file_player,
            episode_cache,
            active_player: Arc::new(Mutex::new(ActivePlayer::None)),
            current_episode_url: Arc::new(Mutex::new(None)),
            current_channel_id: Arc::new(Mutex::new(None)),
            current_episode_index: Arc::new(Mutex::new(None)),
            current_episodes: Arc::new(Mutex::new(Vec::new())),
            all_programs: Arc::new(Mutex::new(Vec::new())),
            groups_expanded: Arc::new(Mutex::new(HashMap::new())),
            favorites: Arc::new(Mutex::new(favorites)),
            runtime,
        }
    }

    /// Set the active player type
    pub fn set_active_player(&self, player: ActivePlayer) {
        if let Ok(mut p) = self.active_player.lock() {
            *p = player;
        }
    }

    /// Get the current active player type
    pub fn get_active_player(&self) -> ActivePlayer {
        self.active_player
            .lock()
            .ok()
            .map(|p| *p)
            .unwrap_or(ActivePlayer::None)
    }

    /// Stop all players
    pub async fn stop_all_players(&self) {
        self.channel_pool.stop_all().await;
        self.file_player.stop();
        self.streaming_player.stop().await;
    }

    /// Set volume on all players
    pub async fn set_volume(&self, volume: f32) {
        self.channel_pool.set_volume(volume).await;
        self.file_player.set_volume(volume);
        self.streaming_player.set_volume(volume);
    }

    /// Pause the currently active player
    pub async fn pause_active(&self) {
        match self.get_active_player() {
            ActivePlayer::FilePlayer => self.file_player.pause(),
            ActivePlayer::Streaming => self.streaming_player.pause(),
            ActivePlayer::ChannelPool => self.channel_pool.pause().await,
            ActivePlayer::None => {}
        }
    }

    /// Resume the currently active player
    pub async fn resume_active(&self) {
        match self.get_active_player() {
            ActivePlayer::FilePlayer => self.file_player.resume(),
            ActivePlayer::Streaming => self.streaming_player.resume(),
            ActivePlayer::ChannelPool => self.channel_pool.resume().await,
            ActivePlayer::None => {}
        }
    }

    /// Set current episode URL for seeking
    pub fn set_current_episode_url(&self, url: Option<String>) {
        if let Ok(mut current) = self.current_episode_url.lock() {
            *current = url;
        }
    }

    /// Get current episode URL
    pub fn get_current_episode_url(&self) -> Option<String> {
        self.current_episode_url.lock().ok().and_then(|u| u.clone())
    }

    /// Set current channel ID (for program polling)
    pub fn set_current_channel_id(&self, id: Option<i32>) {
        if let Ok(mut current) = self.current_channel_id.lock() {
            *current = id;
        }
    }

    /// Get current channel ID
    pub fn get_current_channel_id(&self) -> Option<i32> {
        self.current_channel_id.lock().ok().and_then(|id| *id)
    }

    /// Set current episode index (for auto-play next)
    pub fn set_current_episode_index(&self, index: Option<usize>) {
        if let Ok(mut current) = self.current_episode_index.lock() {
            *current = index;
        }
    }

    /// Get current episode index
    pub fn get_current_episode_index(&self) -> Option<usize> {
        self.current_episode_index.lock().ok().and_then(|idx| *idx)
    }

    /// Set the current episodes list (for auto-play)
    pub fn set_current_episodes(&self, episodes: Vec<crate::EpisodeItem>) {
        if let Ok(mut current) = self.current_episodes.lock() {
            *current = episodes;
        }
    }

    /// Get episode at index (for auto-play next)
    pub fn get_episode_at(&self, index: usize) -> Option<crate::EpisodeItem> {
        self.current_episodes
            .lock()
            .ok()
            .and_then(|eps| eps.get(index).cloned())
    }

    /// Get total episode count
    pub fn get_episode_count(&self) -> usize {
        self.current_episodes
            .lock()
            .ok()
            .map(|eps| eps.len())
            .unwrap_or(0)
    }

    /// Spawn an async task on the runtime
    pub fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(future);
    }
}
