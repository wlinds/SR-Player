// Seek handling for episode and live radio DVR playback

use crate::app_state::AppState;
use crate::core::models::ActivePlayer;
use crate::core::utils::calculate_program_position;
use crate::MainWindow;

/// Context for seeking - gathered from UI before async operations
pub struct SeekContext {
    pub position: f32,
    pub playback_duration: f32,
    pub program_duration: f32,
    pub program_start_time: String,
    pub program_end_time: String,
    pub episode_url: Option<String>,
}

impl SeekContext {
    /// Gather seek context from UI state
    pub fn from_ui(ui: &MainWindow, position: f32, state: &AppState) -> Self {
        let program = ui.get_current_program();
        Self {
            position,
            playback_duration: ui.get_playback_duration(),
            program_duration: ui.get_program_duration(),
            program_start_time: program.start_time.to_string(),
            program_end_time: program.end_time.to_string(),
            episode_url: state.get_current_episode_url(),
        }
    }

    pub fn is_episode(&self) -> bool {
        self.playback_duration > 0.0
    }

    pub fn is_live_radio(&self) -> bool {
        self.program_duration > 0.0 && !self.is_episode()
    }
}

/// Handle seeking in a downloaded episode
pub async fn seek_episode(
    ctx: &SeekContext,
    state: &AppState,
    ui_weak: &slint::Weak<MainWindow>,
) {
    let Some(ref episode_url) = ctx.episode_url else {
        println!("No episode currently playing");
        return;
    };

    // Check if episode is downloaded
    if !state.episode_cache.is_downloaded(episode_url).await {
        println!("Episode not downloaded yet, cannot seek");
        return;
    }

    // Get downloaded data
    let Some(data) = state.episode_cache.get(episode_url).await else {
        println!("Failed to get downloaded data");
        return;
    };

    // Stop other players
    state.streaming_player.stop().await;
    state.channel_pool.stop_all().await;

    // Play from downloaded file at seek position
    let position = ctx.position;
    match state.file_player.play_from_bytes_at_position(data, position).await {
        Ok(_) => {
            state.set_active_player(ActivePlayer::FilePlayer);
            let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                ui.set_playback_position(position);
                ui.set_is_playing(true);
            });
        }
        Err(e) => {
            eprintln!("Failed to seek: {}", e);
            let _ = ui_weak.upgrade_in_event_loop(|ui| {
                ui.set_is_loading(false);
            });
        }
    }
}

/// Handle seeking in live radio DVR buffer
pub async fn seek_live_radio(
    ctx: &SeekContext,
    state: &AppState,
    ui_weak: &slint::Weak<MainWindow>,
) {
    if ctx.program_duration == 0.0 {
        println!("No program duration available, cannot seek");
        return;
    }

    let position = ctx.position;

    // Update live edge state
    let seeking_to_live = position >= (ctx.program_duration - 5.0);
    state.channel_pool.set_at_live_edge(seeking_to_live).await;

    // Get buffer time range
    let (buffer_oldest, buffer_newest) = state.channel_pool.get_buffer_time_range().await;
    let buffer_depth = buffer_newest - buffer_oldest;

    // Recalculate current live position using Stockholm timezone
    let current_live_pos = calculate_program_position(&ctx.program_start_time, &ctx.program_end_time)
        .map(|(pos, dur)| pos.max(0.0).min(dur))
        .unwrap_or(ctx.program_duration);

    let seekable_start = current_live_pos - buffer_depth;
    let seekable_end = current_live_pos;

    // Clamp position to seekable range
    let clamped_position = position.clamp(seekable_start, seekable_end);

    // Calculate buffer offset
    let seconds_back = current_live_pos - clamped_position;
    let clamped_buffer_position = if (clamped_position - seekable_start).abs() < 0.1 {
        buffer_oldest
    } else {
        buffer_newest - seconds_back
    };

    // Check if seeking to live edge (within 5 seconds)
    let seeking_to_live = (buffer_newest - clamped_buffer_position).abs() < 5.0;

    if seeking_to_live {
        seek_to_live_edge(state, ui_weak, clamped_position).await;
    } else {
        seek_to_buffer_position(state, ui_weak, clamped_position, clamped_buffer_position, ctx.program_duration).await;
    }
}

/// Resume live streaming at the live edge
async fn seek_to_live_edge(
    state: &AppState,
    ui_weak: &slint::Weak<MainWindow>,
    position: f32,
) {
    state.file_player.stop();
    state.channel_pool.resume().await;
    state.set_active_player(ActivePlayer::ChannelPool);

    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        ui.set_playback_position(position);
        ui.set_is_playing(true);
        ui.set_is_live(true);
    });
}

/// Play from DVR buffer at a specific position (time-shifted)
async fn seek_to_buffer_position(
    state: &AppState,
    ui_weak: &slint::Weak<MainWindow>,
    program_position: f32,
    buffer_position: f32,
    program_duration: f32,
) {
    let buffer_data = state.channel_pool.get_buffer_data_from_offset(buffer_position).await;

    match buffer_data {
        Ok(data) if !data.is_empty() => {
            state.file_player.stop();
            state.channel_pool.pause().await;

            match state.file_player.play_from_bytes_at_position(bytes::Bytes::from(data), 0.0).await {
                Ok(_) => {
                    state.set_active_player(ActivePlayer::FilePlayer);

                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.set_playback_position(program_position);
                        ui.set_is_playing(true);
                        ui.set_is_live(false);
                    });

                    // Monitor for playback completion
                    spawn_playback_monitor(state.clone(), ui_weak.clone());
                }
                Err(e) => {
                    eprintln!("Failed to start time-shifted playback: {}", e);
                    state.channel_pool.resume().await;

                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        ui.set_playback_position(program_duration);
                        ui.set_is_playing(true);
                        ui.set_is_live(true);
                    });
                }
            }
        }
        Ok(_) => {
            println!("No buffered data available");
            state.channel_pool.resume().await;
        }
        Err(e) => {
            eprintln!("Failed to get buffer data: {}", e);
            state.channel_pool.resume().await;
        }
    }
}

/// Monitor time-shifted playback and auto-switch to live when finished
fn spawn_playback_monitor(state: AppState, ui_weak: slint::Weak<MainWindow>) {
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

            if state.get_active_player() != ActivePlayer::FilePlayer {
                break;
            }

            if state.file_player.is_finished().await {
                println!("Time-shifted playback finished - switching back to live");

                state.file_player.stop();
                state.channel_pool.resume().await;
                state.set_active_player(ActivePlayer::ChannelPool);

                let _ = ui_weak.upgrade_in_event_loop(|ui| {
                    ui.set_is_live(true);
                    ui.set_is_playing(true);
                });

                break;
            }
        }
    });
}
