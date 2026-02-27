// SR Player - Sveriges Radio Streaming Application
//
// A native desktop application for streaming Sveriges Radio channels and podcasts.
// Features:
// - Live radio streaming with gapless playback (no gaps between tracks)
// - Browse and play podcast episodes (25 most recent per program)
// - Organized tabs: Live channels, Podcasts, News, Music
// - Automatic program info updates (polls every 30 seconds)
// - Stockholm timezone support (handles CET/CEST automatically)
//
// ARCHITECTURE:
// - UI: Slint (declarative UI framework)
// - Audio: Rodio + Symphonia (gapless AAC/MP3 streaming)
// - Network: Reqwest (blocking HTTP client)
// - Async: Tokio runtime (for concurrent operations)

// Hide console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// MODULE DECLARATIONS
mod app_state;
mod callbacks;
mod core;
mod localization;
mod translations;

// Use statements
use app_state::AppState;
use core::api::SrApiClient;
use core::channel_pool::ChannelPool;
use core::episode_cache::EpisodeCache;
use core::favorites::Favorites;
use core::file_player_send_safe::SendSafeFilePlayer;
use core::gapless_send_safe::SendSafeGaplessPlayer;
use core::settings::Settings;

use slint::{Model, ModelRc, VecModel};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

// Include Slint UI
slint::include_modules!();

// ============================================================================
// DATA INITIALIZATION
// ============================================================================

/// Initialize channels from API and return both channel map and UI model
fn initialize_channels(
    channels: &[core::models::Channel],
    favorites: &Favorites,
) -> (HashMap<i32, String>, ModelRc<ChannelItem>) {
    let channel_map: HashMap<i32, String> = channels
        .iter()
        .filter_map(|ch| i32::try_from(ch.id).ok().map(|id| (id, ch.name.clone())))
        .collect();

    let channel_items: Vec<ChannelItem> = channels
        .iter()
        .filter_map(|channel| {
            i32::try_from(channel.id).ok().map(|id| ChannelItem {
                id,
                name: channel.name.clone().into(),
                stream_url: channel.live_audio.url.clone().into(),
                is_favorite: favorites.is_channel_favorite(id),
            })
        })
        .collect();

    (
        channel_map,
        ModelRc::from(Rc::new(VecModel::from(channel_items))),
    )
}

// ============================================================================
// MAIN FUNCTION
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logger
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    println!("Starting SR Player...");

    // Create tokio runtime
    let runtime = tokio::runtime::Runtime::new()?;
    let _runtime_guard = runtime.enter();

    println!("Tokio runtime initialized");

    // ========================================================================
    // STEP 1: CREATE UI WINDOW
    // ========================================================================

    let ui = MainWindow::new()?;
    println!("UI window created");

    // ========================================================================
    // STEP 2: CREATE BACKEND COMPONENTS
    // ========================================================================

    let api_client = SrApiClient::new()?;
    let channel_pool = ChannelPool::new().expect("Failed to create channel pool");
    let streaming_player = Arc::new(SendSafeGaplessPlayer::new()?);
    let file_player = Arc::new(SendSafeFilePlayer::new()?);
    let episode_cache = EpisodeCache::new();
    let favorites = Favorites::load();
    let settings = Settings::load();

    println!("Favorites and settings loaded from disk");

    // Bundle all shared state
    let app_state = AppState::new(
        channel_pool,
        streaming_player,
        file_player,
        episode_cache,
        favorites,
        settings,
        runtime.handle().clone(),
    );

    // Initialize volume
    runtime.block_on(async {
        app_state.set_volume(0.25).await;
    });

    // Set initial language from saved settings
    let saved_language = app_state.get_language();

    // Update all translation properties in the UI
    callbacks::update_all_translations(&ui, saved_language);

    // Set the language index in the Settings dropdown to match saved language
    let language_index = match saved_language {
        localization::Language::English => 0,
        localization::Language::Swedish => 1,
        localization::Language::Arabic => 2,
    };
    ui.set_selected_language_index(language_index);

    println!("Backend components initialized");

    // ========================================================================
    // STEP 3: FETCH AND DISPLAY CHANNELS
    // ========================================================================

    println!("Fetching channels from Sveriges Radio API...");
    let channels = api_client.get_channels()?;
    println!("Fetched {} channels", channels.len());

    let (channel_map, channels_model) = {
        let favorites_lock = app_state.favorites.lock().unwrap();
        initialize_channels(&channels, &favorites_lock)
    };
    ui.set_channels(channels_model.clone());

    // Set up initial favorite channels list
    {
        let favorites_lock = app_state.favorites.lock().unwrap();
        let favorite_channels: Vec<ChannelItem> = (0..channels_model.row_count())
            .filter_map(|i| channels_model.row_data(i))
            .filter(|ch| favorites_lock.is_channel_favorite(ch.id))
            .collect();
        ui.set_favorite_channels(ModelRc::from(Rc::new(VecModel::from(favorite_channels))));
    }
    println!("Channels loaded into UI");

    // ========================================================================
    // STEP 4: SET UP UI CALLBACKS
    // ========================================================================

    callbacks::setup_callbacks(&ui, &app_state, &api_client, &channel_map);
    println!("UI callbacks configured");

    // ========================================================================
    // STEP 5: START BACKGROUND TASKS
    // ========================================================================

    callbacks::start_program_update_task(ui.as_weak(), api_client.clone(), app_state.clone());
    println!("Periodic program update task started");

    // ========================================================================
    // STEP 6: RUN THE APPLICATION
    // ========================================================================

    println!("Starting UI event loop...");
    println!("\nSR Player is running!");
    println!("Select a channel from the list to start playing.\n");

    ui.run()?;

    println!("Application closed");
    Ok(())
}
