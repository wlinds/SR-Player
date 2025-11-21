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
//
// PROGRAM FLOW:
// 1. Initialize Tokio runtime and logging
// 2. Load Slint UI components
// 3. Create API client and audio player
// 4. Fetch channels from SR API
// 5. Set up UI callbacks (event handlers)
// 6. Start periodic program update task
// 7. Show window and run event loop

// Hide console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// MODULE DECLARATIONS
// Like 'import' in Python or JavaScript
// These tell Rust where to find our code modules
mod core; // This loads src/core/mod.rs

// Use statements bring items into scope
// Like: from core.api import SrApiClient in Python
// Or: import { SrApiClient } from './core/api' in JavaScript
use core::api::SrApiClient;
use core::channel_pool::ChannelPool;
use core::episode_cache::EpisodeCache;
use core::file_player_send_safe::SendSafeFilePlayer;
use core::gapless_send_safe::SendSafeGaplessPlayer;
use core::utils::{bytes_to_slint_image, fetch_image_bytes, parse_sr_date_to_time};

// Import Slint types
use slint::{Model, ModelRc, VecModel};

// Standard library imports
// Arc = Atomic Reference Counted (thread-safe reference counting)
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

// INCLUDE SLINT UI
// This macro includes the compiled Slint code from build.rs
// It's similar to code generation in other frameworks
slint::include_modules!();

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================
/// Macro to reduce duplication in tab loading callbacks
/// Handles lazy loading of programs with caching
macro_rules! setup_tab_loader {
    (
        ui = $ui:expr,
        runtime = $runtime:expr,
        api_client = $api:expr,
        all_programs = $all_programs:expr,
        groups_expanded = $groups_expanded:expr,
        callback = $callback:ident,
        filter = $filter_fn:expr,
        set_model = $set_model:expr,
        set_loaded = $set_loaded:expr
    ) => {{
        let ui_weak = $ui.as_weak();
        let runtime_handle = $runtime.handle().clone();
        let api_client_clone = $api.clone();
        let all_programs_clone = $all_programs.clone();
        let groups_expanded_clone = $groups_expanded.clone();

        $ui.$callback(move || {
            let ui = ui_weak.upgrade().unwrap();

            // Check if data is already loaded
            let programs_available = {
                let all_programs_lock = all_programs_clone.lock().unwrap();
                !all_programs_lock.is_empty()
            };

            if programs_available {
                // Use cached data
                let all_programs_lock = all_programs_clone.lock().unwrap();
                let groups_expanded_lock = groups_expanded_clone.lock().unwrap();
                let program_items = $filter_fn(&all_programs_lock, &groups_expanded_lock);
                let programs_model = Rc::new(VecModel::from(program_items));
                $set_model(&ui, ModelRc::from(programs_model));
            } else {
                // Fetch fresh data
                let ui_clone = ui_weak.clone();
                let api_clone = api_client_clone.clone();
                let all_programs_clone2 = all_programs_clone.clone();
                let groups_expanded_clone2 = groups_expanded_clone.clone();

                runtime_handle.spawn(async move {
                    match core::podcast::fetch_programs_with_podcasts(&api_clone).await {
                        Ok(programs) => {
                            // Cache the programs
                            {
                                let mut all_programs_lock = all_programs_clone2.lock().unwrap();
                                *all_programs_lock = programs.clone();
                            }

                            // Update UI
                            if let Err(e) = ui_clone.upgrade_in_event_loop(move |ui| {
                                let groups_expanded_lock = groups_expanded_clone2.lock().unwrap();
                                let program_items = $filter_fn(&programs, &groups_expanded_lock);
                                let programs_model = Rc::new(VecModel::from(program_items));
                                $set_model(&ui, ModelRc::from(programs_model));
                                $set_loaded(&ui);
                            }) {
                                eprintln!("Failed to update UI with programs: {:?}", e);
                            }
                        }
                        Err(e) => eprintln!("Failed to fetch programs: {}", e),
                    }
                });
            }
        });
    }};
}

/// Macro to reduce duplication in episode fetching callbacks
macro_rules! setup_episode_fetcher {
    (
        ui = $ui:expr,
        runtime = $runtime:expr,
        api_client = $api:expr,
        callback = $callback:ident,
        switch_tab = $switch_tab:expr
    ) => {{
        let ui_weak = $ui.as_weak();
        let runtime_handle = $runtime.handle().clone();
        let api_client_clone = $api.clone();

        $ui.$callback(move |program_id| {
            let ui_clone = ui_weak.clone();
            let api_clone = api_client_clone.clone();

            runtime_handle.spawn(async move {
                match core::podcast::fetch_episodes_for_program(&api_clone, program_id as u32).await
                {
                    Ok((episode_items, program_name)) => {
                        if let Err(e) = ui_clone.upgrade_in_event_loop(move |ui| {
                            let episodes_model = Rc::new(VecModel::from(episode_items));
                            ui.set_episodes(ModelRc::from(episodes_model));
                            ui.set_selected_program_name(program_name.into());
                            ui.set_selected_program_id(program_id);
                            ui.set_show_episodes_view(true);
                            if $switch_tab {
                                ui.set_current_tab(1); // Switch to podcasts tab
                            }
                        }) {
                            eprintln!("Failed to update UI with episodes: {:?}", e);
                        }
                    }
                    Err(e) => eprintln!("Failed to fetch episodes: {}", e),
                }
            });
        });
    }};
}

/// Helper to fetch program info from schedule for a channel
async fn fetch_program_info_for_channel(
    api_client: &SrApiClient,
    channel_id: i32,
) -> Option<core::models::ScheduledEpisode> {
    let api_clone = api_client.clone();
    tokio::task::spawn_blocking(move || match api_clone.get_schedule_right_now() {
        Ok(schedule) => schedule
            .channels
            .iter()
            .find(|ch| ch.id == channel_id as u32)
            .and_then(|ch| ch.current_episode.clone()),
        Err(e) => {
            eprintln!("Failed to fetch schedule: {}", e);
            None
        }
    })
    .await
    .ok()
    .flatten()
}

/// Helper to create ProgramInfo from a ScheduledEpisode
fn program_info_from_episode(
    episode: &core::models::ScheduledEpisode,
    show_image: slint::Image,
) -> ProgramInfo {
    let start_time = parse_sr_date_to_time(&episode.start_time_utc);
    let end_time = parse_sr_date_to_time(&episode.end_time_utc);
    let program_id = episode.program.as_ref().map(|p| p.id).unwrap_or(0);

    ProgramInfo {
        title: episode.title.clone().into(),
        description: episode.description.clone().unwrap_or_default().into(),
        start_time: start_time.into(),
        end_time: end_time.into(),
        show_image,
        program_id: program_id as i32,
        has_podcasts: false,
    }
}

/// Initialize channels from API and return both channel map and UI model
fn initialize_channels(
    channels: &[core::models::Channel],
) -> (HashMap<i32, String>, ModelRc<ChannelItem>) {
    // Create HashMap for O(1) channel name lookups
    let channel_map: HashMap<i32, String> = channels
        .iter()
        .filter_map(|ch| i32::try_from(ch.id).ok().map(|id| (id, ch.name.clone())))
        .collect();

    // Convert to UI model
    let channel_items: Vec<ChannelItem> = channels
        .iter()
        .filter_map(|channel| {
            i32::try_from(channel.id).ok().map(|id| ChannelItem {
                id,
                name: channel.name.clone().into(),
                stream_url: channel.live_audio.url.clone().into(),
            })
        })
        .collect();

    (
        channel_map,
        ModelRc::from(Rc::new(VecModel::from(channel_items))),
    )
}

/// Refresh program list based on current tab
fn refresh_program_list(
    ui: &MainWindow,
    all_programs: &[core::models::Program],
    groups_expanded: &HashMap<i32, bool>,
) {
    let current_tab = ui.get_current_tab();
    if current_tab == 2 {
        // News tab
        let items = core::podcast::programs_to_items_news(all_programs, groups_expanded);
        ui.set_news_programs(ModelRc::from(Rc::new(VecModel::from(items))));
    } else if current_tab == 1 {
        // Podcasts tab
        let items = core::podcast::programs_to_items_podcasts(all_programs);
        ui.set_programs(ModelRc::from(Rc::new(VecModel::from(items))));
    }
}

/// Fetch and update program information for a given channel ID
/// Returns the current program title to detect changes
async fn update_program_info(
    ui_weak: &slint::Weak<MainWindow>,
    api_client: &SrApiClient,
    channel_id: i32,
) -> Option<String> {
    let episode = fetch_program_info_for_channel(api_client, channel_id).await?;
    let program_title = episode.title.clone();

    // Download image in background
    let image_url = episode.social_image.clone().unwrap_or_default();
    let image_bytes = fetch_image_bytes(image_url).await;

    // Update UI (must create Image inside event loop since Image is not Send)
    ui_weak
        .upgrade_in_event_loop(move |ui| {
            let show_image = image_bytes
                .and_then(bytes_to_slint_image)
                .unwrap_or_default();
            let program_info = program_info_from_episode(&episode, show_image);
            ui.set_current_program(program_info);
        })
        .ok();

    Some(program_title)
}

/// Helper to find an episode by ID in a Slint model
fn find_episode_in_model(episodes: &ModelRc<EpisodeItem>, episode_id: i32) -> Option<EpisodeItem> {
    for i in 0..episodes.row_count() {
        if let Some(episode) = episodes.row_data(i) {
            if episode.id == episode_id {
                return Some(episode);
            }
        }
    }
    None
}

// ============================================================================
// MAIN FUNCTION - PROGRAM ENTRY POINT
// ============================================================================

// In Rust, main() returns Result for error handling
// Result<(), Box<dyn std::error::Error>> means:
// - Ok(()) if successful (the () is like 'void' or 'None')
// - Err(error) if something goes wrong
//
// Note: We DON'T use #[tokio::main] because Slint needs to run on the main thread
// Instead, we create a tokio runtime manually in the background
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize the logger (for debug/info messages)
    // RUST_LOG=debug cargo run to see debug logs
    // Like console.log() in JavaScript or print() in Python, but better
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info) // Show INFO logs by default
        .init();

    println!("Starting SR Player...");

    // Create a tokio runtime for async operations
    // We run it in a separate thread so Slint can have the main thread
    let runtime = tokio::runtime::Runtime::new()?;
    let _runtime_guard = runtime.enter(); // Enter the runtime context

    println!("Tokio runtime initialized");

    // ========================================================================
    // STEP 1: CREATE UI WINDOW
    // ========================================================================

    // Create the main window from our Slint UI definition
    // Like: const app = new App() in React or root = tk.Tk() in Tkinter
    let ui = MainWindow::new()?;

    println!("UI window created");

    // ========================================================================
    // STEP 2: CREATE BACKEND COMPONENTS
    // ========================================================================

    // Create the API client (for fetching SR data)
    let api_client = SrApiClient::new()?;

    // Create multi-channel streaming pool
    // Keeps up to 3 channels streaming in the background for instant switching
    // ChannelPool is already Clone + Send + Sync, no need for Arc wrapper
    let channel_pool = ChannelPool::new().expect("Failed to create channel pool");

    // Create episode cache for background downloads
    let episode_cache = EpisodeCache::new();

    // Create streaming player for podcast instant playback
    let streaming_player = Arc::new(SendSafeGaplessPlayer::new()?);

    // Create file player for seekable playback (when episode is downloaded)
    let file_player = Arc::new(SendSafeFilePlayer::new()?);

    // Track current episode URL for seeking
    let current_episode_url: Arc<std::sync::Mutex<Option<String>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Track which player is active (true = file_player, false = streaming/channel_pool)
    let using_file_player: Arc<std::sync::Mutex<bool>> = Arc::new(std::sync::Mutex::new(false));

    // Initialize volume to match UI default (0.5 = 50%)
    // Note: UI uses logarithmic scale, so 0.5 slider = 0.25 actual volume
    runtime.block_on(async {
        channel_pool.set_volume(0.25).await;
    });
    file_player.set_volume(0.25);

    println!("Backend components initialized");

    // ========================================================================
    // STEP 3: FETCH AND DISPLAY CHANNELS
    // ========================================================================

    // Fetch and initialize channels
    println!("Fetching channels from Sveriges Radio API...");
    let channels = api_client.get_channels()?;
    println!("Fetched {} channels", channels.len());

    let (channel_map, channels_model) = initialize_channels(&channels);
    ui.set_channels(channels_model);
    println!("Channels loaded into UI");

    // ========================================================================
    // SHARED STATE FOR GROUP MANAGEMENT AND INFINITE SCROLLING
    // ========================================================================

    // Track which program groups are expanded/collapsed
    let groups_expanded = Arc::new(std::sync::Mutex::new(HashMap::<i32, bool>::new()));

    // Cache all programs for infinite scrolling
    let all_programs = Arc::new(std::sync::Mutex::new(Vec::<core::models::Program>::new()));

    // Track the currently playing channel ID for periodic program updates
    let current_channel_id = Arc::new(std::sync::Mutex::new(Option::<i32>::None));

    // ========================================================================
    // STEP 4: SET UP UI CALLBACKS (EVENT HANDLERS)
    // ========================================================================

    // Clone references for use in callbacks
    // In Rust, closures (like arrow functions) can "capture" variables
    // We need to clone these Rc references before moving them into closures
    //
    // Like:
    // JavaScript: const playerCopy = player;
    // Python: player_copy = player (but for closures)

    // ========================================================================
    // UI EVENT HANDLERS
    // ========================================================================
    // Each callback below handles a specific user interaction.
    // The pattern is: clone state -> create closure -> register callback

    // CALLBACK 1: Channel Selection Handler
    // Triggered when user clicks on a radio channel
    {
        let ui_weak = ui.as_weak();
        let pool = channel_pool.clone();
        let file_player_clone = file_player.clone();
        let streaming_player_clone = streaming_player.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();
        let api_client_clone = api_client.clone();
        let channel_map_clone = channel_map.clone();
        let current_channel_id_clone = current_channel_id.clone();

        ui.on_channel_selected(move |channel_id, stream_url| {
            println!("Channel selected: ID={}, URL={}", channel_id, stream_url);

            // Store the current channel ID for periodic program updates
            if let Ok(mut current_id) = current_channel_id_clone.lock() {
                *current_id = Some(channel_id);
            }

            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_is_loading(true);

            // Clear episode duration for live radio
            // program-duration will be set by the live position update callback
            ui.set_playback_duration(0.0);
            ui.set_playback_position(0.0);
            ui.set_program_duration(0.0); // Will be calculated when program info arrives

            // Get channel name using O(1) HashMap lookup
            let channel_name = channel_map_clone
                .get(&channel_id)
                .cloned()
                .unwrap_or_else(|| String::from("Unknown"));
            ui.set_current_channel_name(channel_name.into());

            let stream_url = stream_url.to_string();
            let ui_weak_clone = ui_weak.clone();
            let api_clone = api_client_clone.clone();
            let pool_clone = pool.clone();
            let file_player_for_stop = file_player_clone.clone();
            let streaming_player_for_stop = streaming_player_clone.clone();
            let using_file_for_stop = using_file_player_clone.clone();

            // Spawn async task to stop file player, fetch program info, and switch channel
            runtime_handle.spawn(async move {
                // Stop both file player and streaming player (in case an episode is playing)
                file_player_for_stop.stop();
                streaming_player_for_stop.stop().await;
                if let Ok(mut using_file) = using_file_for_stop.lock() {
                    *using_file = false;
                }
                // Fetch and display current program information
                if let Some(episode) = fetch_program_info_for_channel(&api_clone, channel_id).await
                {
                    let image_bytes =
                        fetch_image_bytes(episode.social_image.clone().unwrap_or_default()).await;
                    let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                        let show_image = image_bytes
                            .and_then(bytes_to_slint_image)
                            .unwrap_or_default();
                        ui.set_current_program(program_info_from_episode(&episode, show_image));

                        // Trigger initial live position update
                        ui.invoke_update_live_position();
                    });
                }

                // Switch to the channel (instant if already in pool!)
                let start = std::time::Instant::now();
                match pool_clone.switch_to_channel(channel_id, stream_url).await {
                    Ok(()) => {
                        let elapsed = start.elapsed().as_millis();
                        println!("Channel switch completed in {}ms", elapsed);

                        let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                            ui.set_is_playing(true);
                            ui.set_is_loading(false);
                            ui.set_is_live(true);
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to switch channel: {}", e);
                        let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                            ui.set_is_loading(false);
                        });
                    }
                }
            });
        });
    }

    // CALLBACK 2: Play/Pause button
    {
        let ui_weak = ui.as_weak();
        let channel_pool_clone = channel_pool.clone();
        let file_player_clone = file_player.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();

        ui.on_play_pause_clicked(move || {
            let ui_clone = ui_weak.upgrade().unwrap();
            let is_playing = ui_clone.get_is_playing();

            // Check which player is active
            let is_using_file_player = using_file_player_clone
                .lock()
                .ok()
                .map(|f| *f)
                .unwrap_or(false);

            if is_playing {
                // Pause playback
                println!("Pausing playback");
                if is_using_file_player {
                    file_player_clone.pause();
                } else {
                    let pool = channel_pool_clone.clone();
                    runtime_handle.spawn(async move {
                        pool.pause().await;
                    });
                }
                ui_clone.set_is_playing(false);
            } else {
                // Resume playback (or do nothing if no channel selected)
                let channel_name = ui_clone.get_current_channel_name();

                if channel_name != "No channel selected" {
                    println!("Resuming playback");
                    if is_using_file_player {
                        file_player_clone.resume();
                    } else {
                        let pool = channel_pool_clone.clone();
                        runtime_handle.spawn(async move {
                            pool.resume().await;
                        });
                    }
                    ui_clone.set_is_playing(true);
                } else {
                    println!("No channel selected - cannot resume playback");
                }
            }
        });
    }

    // CALLBACK 2b: Volume changed
    {
        let channel_pool_clone = channel_pool.clone();
        let file_player_clone = file_player.clone();
        let runtime_handle = runtime.handle().clone();

        ui.on_volume_changed(move |volume| {
            let pool = channel_pool_clone.clone();
            runtime_handle.spawn(async move {
                pool.set_volume(volume).await;
            });
            file_player_clone.set_volume(volume);
        });
    }

    // CALLBACK 2c: Seek to position (works for episodes and live radio DVR)
    {
        let ui_weak = ui.as_weak();
        let episode_cache_clone = episode_cache.clone();
        let current_episode_url_clone = current_episode_url.clone();
        let streaming_player_clone = streaming_player.clone();
        let file_player_clone = file_player.clone();
        let channel_pool_clone = channel_pool.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();

        ui.on_seek_to_position(move |position| {
            println!("=== SEEK CALLBACK TRIGGERED: position={:.1}s ===", position);

            let Some(ui_handle) = ui_weak.upgrade() else {
                println!("UI no longer available");
                return;
            };

            let playback_duration = ui_handle.get_playback_duration();
            let program_duration = ui_handle.get_program_duration();
            println!("playback_duration={:.1}, program_duration={:.1}", playback_duration, program_duration);

            let is_episode = playback_duration > 0.0;
            let is_live_radio = program_duration > 0.0 && !is_episode;

            println!("is_episode={}, is_live_radio={}", is_episode, is_live_radio);

            if is_episode {
                println!("Entering episode seek logic");
                // Episode seeking (existing logic)
                let episode_url_opt = {
                    current_episode_url_clone
                        .lock()
                        .ok()
                        .and_then(|url| url.clone())
                };

                let Some(episode_url) = episode_url_opt else {
                    println!("No episode currently playing");
                    return;
                };

                let cache_clone = episode_cache_clone.clone();
                let player_clone = file_player_clone.clone();
                let streaming_clone = streaming_player_clone.clone();
                let pool_clone = channel_pool_clone.clone();
                let using_file_clone = using_file_player_clone.clone();
                let ui_weak_clone = ui_weak.clone();

                runtime_handle.spawn(async move {
                    // Check if episode is downloaded
                    if !cache_clone.is_downloaded(&episode_url).await {
                        println!("Episode not downloaded yet, cannot seek");
                        return;
                    }

                    // Get downloaded data
                    let Some(data) = cache_clone.get(&episode_url).await else {
                        println!("Failed to get downloaded data");
                        return;
                    };

                    // Stop streaming player and channel pool (file player will handle its own stop in play_from_bytes_at_position)
                    streaming_clone.stop().await;
                    pool_clone.stop_all().await;

                    // Play from downloaded file at seek position
                    println!("Playing from downloaded file at {}s", position);

                    match player_clone
                        .play_from_bytes_at_position(data, position)
                        .await
                    {
                        Ok(_) => {
                            println!("Successfully seeked to {}s", position);

                            // Mark that we're now using file player
                            if let Ok(mut using_file) = using_file_clone.lock() {
                                *using_file = true;
                            }

                            // Update UI
                            let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                                ui.set_playback_position(position);
                                ui.set_is_playing(true);
                            });
                        }
                        Err(e) => {
                            eprintln!("Failed to seek: {}", e);
                            let _ = ui_weak_clone.upgrade_in_event_loop(|ui| {
                                ui.set_is_loading(false);
                            });
                        }
                    }
                });
            } else if is_live_radio {
                // Live radio DVR seeking (hybrid approach)
                // Switch to file player for time-shifted playback while keeping live stream running
                let pool_clone2 = channel_pool_clone.clone();
                let file_player_clone2 = file_player_clone.clone();
                let using_file_clone2 = using_file_player_clone.clone();

                // Get program info before spawning (while we have ui_handle)
                let program_duration = ui_handle.get_program_duration();
                let current_live_position = ui_handle.get_playback_position();
                let program = ui_handle.get_current_program();

                // Clone weak reference for async task
                let ui_weak_clone = ui_weak.clone();

                runtime_handle.spawn(async move {
                    println!("Seeking in live radio DVR buffer to program position {}s", position);

                    if program_duration == 0.0 {
                        println!("No program duration available, cannot seek");
                        return;
                    }

                    // DVR LOGIC:
                    // - Single buffer continuously tracks live edge (last 30 minutes)
                    // - We can seek anywhere within that 30-minute window
                    // - Track state (live vs time-shifted) for UI indicators

                    // Detect seek direction relative to live edge
                    let seeking_to_live = position >= (program_duration - 5.0);

                    // Update state based on seek direction
                    if seeking_to_live {
                        pool_clone2.set_at_live_edge(true).await;
                    } else {
                        pool_clone2.set_at_live_edge(false).await;
                    }

                    // Get buffer time range (time since we started recording)
                    let (buffer_oldest, buffer_newest) = pool_clone2.get_buffer_time_range().await;
                    let buffer_depth = buffer_newest - buffer_oldest;

                    println!("Buffer range: {:.1}s - {:.1}s (depth: {:.1}s)", buffer_oldest, buffer_newest, buffer_depth);
                    println!("Program duration: {:.1}s, current position: {:.1}s", program_duration, current_live_position);

                    // KEY INSIGHT: The DVR buffer only contains audio from when we started listening!
                    // We can't seek to arbitrary positions in the program that happened before we tuned in.
                    //
                    // The buffer continuously records as time advances. At any moment:
                    // - program_duration = current live position in the show (keeps increasing)
                    // - buffer contains the last buffer_depth seconds of audio
                    // - We can seek within [program_duration - buffer_depth, program_duration]
                    //
                    // However, we need to get the CURRENT live position, not the snapshot from when callback started.
                    // The periodic update task updates program_duration every 30 seconds, but it may be stale.
                    // For seeking, we should recalculate the current live position based on wall clock time.

                    // Recalculate current live position using the same logic as update_live_position
                    let current_live_pos = if !program.start_time.is_empty() && !program.end_time.is_empty() {
                        // Parse and calculate current position
                        let parse_time = |time_str: &str| -> Option<(i32, i32)> {
                            let parts: Vec<&str> = time_str.split(':').collect();
                            if parts.len() == 2 {
                                let hours = parts[0].parse::<i32>().ok()?;
                                let minutes = parts[1].parse::<i32>().ok()?;
                                Some((hours, minutes))
                            } else {
                                None
                            }
                        };

                        if let (Some((start_h, start_m)), Some((end_h, end_m))) =
                            (parse_time(&program.start_time), parse_time(&program.end_time)) {

                            use chrono::Timelike;
                            let now = chrono::Local::now();
                            let current_h = now.hour() as i32;
                            let current_m = now.minute() as i32;
                            let current_s = now.second() as i32;

                            let start_minutes = start_h * 60 + start_m;
                            let mut end_minutes = end_h * 60 + end_m;
                            if end_minutes < start_minutes {
                                end_minutes += 24 * 60;
                            }
                            let current_minutes = current_h * 60 + current_m;

                            let mut position = (current_minutes - start_minutes) as f32 * 60.0 + current_s as f32;
                            if position < 0.0 {
                                position += 24.0 * 60.0 * 60.0;
                            }
                            let duration = (end_minutes - start_minutes) as f32 * 60.0;
                            position.max(0.0).min(duration)
                        } else {
                            program_duration  // Fallback to UI value
                        }
                    } else {
                        program_duration  // Fallback to UI value
                    };

                    let seekable_start = current_live_pos - buffer_depth;
                    let seekable_end = current_live_pos;  // Can seek up to current live edge

                    println!("Seekable range: {:.1}s - {:.1}s (live position: {:.1}s, buffer depth: {:.1}s)",
                             seekable_start, seekable_end, current_live_pos, buffer_depth);

                    // Clamp position to seekable range
                    let clamped_position = if position < seekable_start {
                        println!("Requested position {:.1}s is before the buffered range (starts at {:.1}s)",
                                 position, seekable_start);
                        println!("Seeking to earliest available position: {:.1}s", seekable_start);
                        seekable_start
                    } else if position > seekable_end {
                        println!("Requested position {:.1}s is beyond the live edge (at {:.1}s)",
                                 position, seekable_end);
                        println!("Seeking to live edge: {:.1}s", seekable_end);
                        seekable_end
                    } else {
                        position
                    };

                    // Calculate buffer offset from the current live edge
                    // The buffer's newest position corresponds to current_live_pos (live edge)
                    // Special case: if we clamped to seekable_start, use buffer_oldest directly
                    let seconds_back = current_live_pos - clamped_position;
                    let clamped_buffer_position = if (clamped_position - seekable_start).abs() < 0.1 {
                        // We clamped to the earliest position, so use buffer_oldest
                        buffer_oldest
                    } else {
                        // Normal case: calculate from live edge
                        buffer_newest - seconds_back
                    };

                    println!("Seeking to program position: {:.1}s (buffer position: {:.1}s, {:.1}s back from live)",
                             clamped_position, clamped_buffer_position, seconds_back);

                    // Check if seeking to live edge (within 5 seconds)
                    let seeking_to_live = (buffer_newest - clamped_buffer_position).abs() < 5.0;

                    if seeking_to_live {
                        println!("Seeking to live edge (position: {:.1}s) - resuming live stream", clamped_position);

                        // Switch back to live streaming
                        file_player_clone2.stop();
                        pool_clone2.resume().await;

                        if let Ok(mut using_file) = using_file_clone2.lock() {
                            *using_file = false;
                        }

                        // Set position to current live position
                        let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                            ui.set_playback_position(clamped_position);
                            ui.set_is_playing(true);
                            ui.set_is_live(true);
                        });
                    } else {
                        println!("Seeking to program position {:.1}s (buffer position {:.1}s) - switching to time-shifted playback",
                                 clamped_position, clamped_buffer_position);

                        // Get buffered data from the DVR using buffer time
                        let buffer_data = pool_clone2.get_buffer_data_from_offset(clamped_buffer_position).await;

                        match buffer_data {
                            Ok(data) if !data.is_empty() => {
                                println!("Got buffer data: {} bytes", data.len());

                                // Stop any previous time-shifted playback before starting new one
                                file_player_clone2.stop();

                                // Pause live stream (keep downloading but pause audio output)
                                pool_clone2.pause().await;

                                // Play buffered data from the seek position using file player
                                // NOTE: The data we got already starts at clamped_buffer_position,
                                // so we play from the beginning of this data (offset 0.0)
                                println!("Playing buffered data starting from buffer position {:.1}s (program position {:.1}s)",
                                         clamped_buffer_position, clamped_position);

                                match file_player_clone2.play_from_bytes_at_position(bytes::Bytes::from(data), 0.0).await {
                                    Ok(_) => {
                                        println!("Time-shifted playback started at program position {:.1}s", clamped_position);

                                        if let Ok(mut using_file) = using_file_clone2.lock() {
                                            *using_file = true;
                                        }

                                        let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                                            ui.set_playback_position(clamped_position);
                                            ui.set_is_playing(true);
                                            ui.set_is_live(false);
                                        });

                                        // Spawn a task to monitor when the buffered audio finishes
                                        // and automatically switch back to live streaming
                                        let file_player_monitor = file_player_clone2.clone();
                                        let pool_monitor = pool_clone2.clone();
                                        let using_file_monitor = using_file_clone2.clone();
                                        let ui_weak_monitor = ui_weak_clone.clone();

                                        tokio::spawn(async move {
                                            // Wait a bit before starting to check
                                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                                            // Check every 100ms if the file player has finished
                                            loop {
                                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                                                // Check if we're still using file player
                                                let still_using_file = {
                                                    if let Ok(using_file) = using_file_monitor.lock() {
                                                        *using_file
                                                    } else {
                                                        break;
                                                    }
                                                };

                                                if !still_using_file {
                                                    // User manually switched back to live or stopped
                                                    break;
                                                }

                                                // Check if the file player's sink is empty (finished playing)
                                                if file_player_monitor.is_finished().await {
                                                    println!("Time-shifted playback finished - automatically switching back to live");

                                                    // Switch back to live streaming
                                                    file_player_monitor.stop();
                                                    pool_monitor.resume().await;

                                                    if let Ok(mut using_file) = using_file_monitor.lock() {
                                                        *using_file = false;
                                                    }

                                                    // Update UI to show we're back at live
                                                    let _ = ui_weak_monitor.upgrade_in_event_loop(move |ui| {
                                                        ui.set_is_live(true);
                                                        ui.set_is_playing(true);
                                                    });

                                                    break;
                                                }
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to start time-shifted playback: {}", e);
                                        eprintln!("Falling back to live stream");
                                        pool_clone2.resume().await;

                                        // Update UI to show we're back at live
                                        let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                                            ui.set_playback_position(program_duration);
                                            ui.set_is_playing(true);
                                            ui.set_is_live(true);
                                        });
                                    }
                                }
                            }
                            Ok(_) => {
                                println!("No buffered data available");
                                pool_clone2.resume().await;
                            }
                            Err(e) => {
                                eprintln!("Failed to get buffer data: {}", e);
                                pool_clone2.resume().await;
                            }
                        }
                    }
                });
            }
        });
    }

    // CALLBACK 2d: Update live radio position based on current time
    {
        let ui_weak = ui.as_weak();

        ui.on_update_live_position(move || {
            let Some(ui) = ui_weak.upgrade() else {
                return;
            };

            // Only update if playing live radio (duration = 0) with program info
            if ui.get_playback_duration() > 0.0 {
                return;
            }

            let program = ui.get_current_program();
            if program.start_time.is_empty() || program.end_time.is_empty() {
                return;
            }

            // Parse times (format: "HH:MM")
            let parse_time = |time_str: &str| -> Option<(i32, i32)> {
                let parts: Vec<&str> = time_str.split(':').collect();
                if parts.len() == 2 {
                    let hours = parts[0].parse::<i32>().ok()?;
                    let minutes = parts[1].parse::<i32>().ok()?;
                    Some((hours, minutes))
                } else {
                    None
                }
            };

            let Some((start_h, start_m)) = parse_time(&program.start_time) else {
                return;
            };
            let Some((end_h, end_m)) = parse_time(&program.end_time) else {
                return;
            };

            // Get current time (in Stockholm timezone - CET/CEST)
            use chrono::Timelike;
            let now = chrono::Local::now();
            let current_h = now.hour() as i32;
            let current_m = now.minute() as i32;
            let current_s = now.second() as i32;

            // Calculate times in minutes since midnight
            let start_minutes = start_h * 60 + start_m;
            let mut end_minutes = end_h * 60 + end_m;

            // Handle midnight wraparound
            if end_minutes < start_minutes {
                end_minutes += 24 * 60;
            }

            let current_minutes = current_h * 60 + current_m;

            // Calculate position and duration
            let duration = (end_minutes - start_minutes) as f32 * 60.0; // seconds
            let mut position = (current_minutes - start_minutes) as f32 * 60.0 + current_s as f32; // seconds (include seconds!)

            // Handle case where current time is after midnight but program started before
            if position < 0.0 {
                position += 24.0 * 60.0 * 60.0; // Add 24 hours in seconds
            }

            // Clamp position to valid range
            position = position.max(0.0).min(duration);

            ui.set_program_duration(duration);
            ui.set_playback_position(position);
        });
    }

    // CALLBACK 3: Podcasts tab clicked - lazy load programs
    setup_tab_loader! {
        ui = ui,
        runtime = runtime,
        api_client = api_client,
        all_programs = all_programs,
        groups_expanded = groups_expanded,
        callback = on_podcasts_tab_clicked,
        filter = |programs, _groups| core::podcast::programs_to_items_podcasts(programs),
        set_model = |ui: &MainWindow, model| ui.set_programs(model),
        set_loaded = |ui: &MainWindow| ui.set_programs_loaded(true)
    }

    // CALLBACK 3b: News tab clicked - lazy load programs with news filtering
    setup_tab_loader! {
        ui = ui,
        runtime = runtime,
        api_client = api_client,
        all_programs = all_programs,
        groups_expanded = groups_expanded,
        callback = on_news_tab_clicked,
        filter = core::podcast::programs_to_items_news,
        set_model = |ui: &MainWindow, model| ui.set_news_programs(model),
        set_loaded = |ui: &MainWindow| ui.set_news_loaded(true)
    }

    // CALLBACK 4: Browse podcasts button (from Live tab's program info)
    setup_episode_fetcher! {
        ui = ui,
        runtime = runtime,
        api_client = api_client,
        callback = on_browse_podcasts_clicked,
        switch_tab = true  // Switch to podcasts tab
    }

    // CALLBACK 5: Program selected in podcasts/news tabs
    setup_episode_fetcher! {
        ui = ui,
        runtime = runtime,
        api_client = api_client,
        callback = on_program_selected,
        switch_tab = false  // Stay on current tab
    }

    // CALLBACK 6: Episode selected for playback
    {
        let ui_weak = ui.as_weak();
        let channel_pool_clone = channel_pool.clone();
        let streaming_player_clone = streaming_player.clone();
        let file_player_clone = file_player.clone();
        let episode_cache_clone = episode_cache.clone();
        let current_episode_url_clone = current_episode_url.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();
        let all_programs_clone = all_programs.clone();
        let current_channel_id_clone = current_channel_id.clone();

        ui.on_episode_selected(move |episode_id| {
            // Stop live program polling
            if let Ok(mut current_id) = current_channel_id_clone.lock() {
                *current_id = None;
            }

            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_is_loading(true);
            ui.set_current_channel_name("Podcast".into());

            let Some(episode) = find_episode_in_model(&ui.get_episodes(), episode_id) else {
                eprintln!("Episode {} not found", episode_id);
                let _ = ui_weak.upgrade_in_event_loop(|ui| ui.set_is_loading(false));
                return;
            };

            // Set episode info (without image initially)
            ui.set_current_program(ProgramInfo {
                title: episode.title.clone(),
                description: episode.description.clone(),
                start_time: "".into(),
                end_time: episode.publish_date.clone(),
                show_image: slint::Image::default(),
                program_id: 0,
                has_podcasts: false,
            });

            // Set playback duration (in seconds)
            println!("Setting playback_duration to: {} seconds", episode.duration);
            ui.set_playback_duration(episode.duration as f32);
            ui.set_playback_position(0.0);
            ui.set_program_duration(0.0); // Clear program duration (this is an episode, not live)

            // Load program image in background if available
            let program_image_url = {
                let all_programs_lock = all_programs_clone.lock().unwrap();
                core::podcast::get_program_image_url(
                    &all_programs_lock,
                    ui.get_selected_program_id(),
                )
            };

            if let Some(image_url) = program_image_url {
                let ui_clone = ui_weak.clone();
                let title = episode.title.clone();
                let desc = episode.description.clone();
                let date = episode.publish_date.clone();

                runtime_handle.spawn(async move {
                    let image_bytes = fetch_image_bytes(image_url).await;
                    let _ = ui_clone.upgrade_in_event_loop(move |ui| {
                        let show_image = image_bytes
                            .and_then(bytes_to_slint_image)
                            .unwrap_or_default();
                        ui.set_current_program(ProgramInfo {
                            title,
                            description: desc,
                            start_time: "".into(),
                            end_time: date,
                            show_image,
                            program_id: 0,
                            has_podcasts: false,
                        });
                    });
                });
            }

            let url = episode.url.to_string();

            // Store current episode URL
            if let Ok(mut current_url) = current_episode_url_clone.lock() {
                *current_url = Some(url.clone());
            }

            // Stop both players, then start new episode playback
            let pool_clone = channel_pool_clone.clone();
            let file_clone = file_player_clone.clone();
            let streaming_for_stop = streaming_player_clone.clone();
            let streaming_for_playback = streaming_player_clone.clone();
            let using_file_clone = using_file_player_clone.clone();
            let episode_cache_for_download = episode_cache_clone.clone();
            let ui_weak_for_download = ui_weak.clone();
            let ui_weak_for_playback = ui_weak.clone();
            let url_for_download = url.clone();
            let url_for_playback = url.clone();

            runtime_handle.spawn(async move {
                // Stop all players first
                pool_clone.stop_all().await;
                file_clone.stop();
                streaming_for_stop.stop().await;

                // Reset player tracking flag
                if let Ok(mut using_file) = using_file_clone.lock() {
                    *using_file = false;
                }

                // Start background download for seeking with progress tracking
                episode_cache_for_download.start_download(
                    url_for_download,
                    move |downloaded, total| {
                        let percent = if total > 0 {
                            (downloaded as f64 / total as f64 * 100.0) as i32
                        } else {
                            0
                        };

                        let ui_for_clear = ui_weak_for_download.clone();
                        let _ = ui_weak_for_download.upgrade_in_event_loop(move |ui| {
                            if downloaded >= total && total > 0 {
                                ui.set_download_status("Downloaded (100%)".into());
                                // Clear after 2 seconds
                                slint::Timer::single_shot(
                                    std::time::Duration::from_secs(2),
                                    move || {
                                        let _ = ui_for_clear.upgrade_in_event_loop(|ui| {
                                            ui.set_download_status("".into());
                                        });
                                    },
                                );
                            } else {
                                ui.set_download_status(
                                    format!("Downloading ({}%)", percent).into(),
                                );
                            }
                        });
                    },
                );

                // Start streaming playback (after stop completes)
                match streaming_for_playback.start_stream(url_for_playback).await {
                    Ok(_) => {
                        let _ = ui_weak_for_playback.upgrade_in_event_loop(|ui| {
                            ui.set_is_playing(true);
                            ui.set_is_loading(false);
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to start stream: {}", e);
                        let _ = ui_weak_for_playback.upgrade_in_event_loop(|ui| {
                            ui.set_is_playing(false);
                            ui.set_is_loading(false);
                        });
                    }
                }
            });
        });
    }

    // CALLBACK 7: Back to programs list (M4)
    {
        let ui_weak = ui.as_weak();

        ui.on_back_to_programs_clicked(move || {
            println!("Back to programs clicked");

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_show_episodes_view(false);
            }
        });
    }

    // CALLBACK 8: Group toggled (expand/collapse P4 group)
    {
        let ui_weak = ui.as_weak();
        let groups_expanded_clone = groups_expanded.clone();
        let all_programs_clone = all_programs.clone();

        ui.on_group_toggled(move |group_id| {
            println!("Group toggled: ID={}", group_id);

            let Some(ui) = ui_weak.upgrade() else { return };

            // Toggle group expansion state
            let mut groups_lock = groups_expanded_clone.lock().unwrap();
            let current = groups_lock.get(&group_id).copied().unwrap_or(false);
            groups_lock.insert(group_id, !current);

            // Refresh program list
            let all_programs_lock = all_programs_clone.lock().unwrap();
            refresh_program_list(&ui, &all_programs_lock, &groups_lock);
        });
    }

    // CALLBACK 9: Load more programs (placeholder for future infinite scrolling)
    {
        let _ui_weak = ui.as_weak();

        ui.on_load_more_programs(move || {
            println!("Load more programs requested");
            // Future enhancement: implement infinite scrolling here
            // For now, all programs are loaded at once with grouping
        });
    }

    // CALLBACK 10: Window dragging (for frameless window)
    {
        let ui_weak = ui.as_weak();

        ui.on_window_moved(move |delta_x, delta_y| {
            if let Some(ui_handle) = ui_weak.upgrade() {
                let window = ui_handle.window();
                let logical_pos = window.position().to_logical(window.scale_factor());
                window.set_position(slint::LogicalPosition::new(
                    logical_pos.x + delta_x,
                    logical_pos.y + delta_y,
                ));
            }
        });
    }

    println!("UI callbacks configured");

    // ========================================================================
    // PERIODIC DVR BUFFER UPDATE TASK
    // ========================================================================

    // Spawn a background task to update DVR buffer depth and live status
    // This runs every 2 seconds to keep the UI updated
    // TODO: Re-enable DVR buffer depth when channel pool supports DVR
    /* DISABLED FOR NOW - DVR features need to be integrated with channel pool
    {
        let ui_weak = ui.as_weak();
        let channel_pool_clone = channel_pool.clone();
        let current_channel_id_clone = current_channel_id.clone();
        let runtime_handle = runtime.handle().clone();

        runtime_handle.spawn(async move {
            loop {
                // Wait 2 seconds between updates
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                // Only update if a channel is playing
                let is_channel_playing = {
                    let current_id = current_channel_id_clone.lock().unwrap();
                    current_id.is_some()
                };

                if is_channel_playing {
                    // Get buffer depth
                    let buffer_depth = streaming_player_clone.get_buffer_depth().await;

                    // Update UI
                    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                        // Only update for live radio (not episodes)
                        if ui.get_playback_duration() == 0.0 && ui.get_program_duration() > 0.0 {
                            ui.set_buffer_depth(buffer_depth);

                            // Don't modify is_live here - it's managed by the seek logic
                            // The Timer will call update_live_position() when is_live is true
                        }
                    });
                }
            }
        });
    }
    */ // END DISABLED DVR BUFFER UPDATE

    // println!("DVR buffer update task started");  // Disabled with DVR features

    // ========================================================================
    // PERIODIC PROGRAM UPDATE TASK
    // ========================================================================

    // Spawn a background task to periodically check for program changes
    // This runs every 30 seconds and updates the UI when a new show starts
    {
        let ui_weak = ui.as_weak();
        let api_client_clone = api_client.clone();
        let current_channel_id_clone = current_channel_id.clone();
        let runtime_handle = runtime.handle().clone();

        runtime_handle.spawn(async move {
            let mut last_program_title: Option<String> = None;

            loop {
                // Wait 30 seconds between checks
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

                // Check if a channel is currently playing
                let channel_id = {
                    let current_id = current_channel_id_clone.lock().unwrap();
                    *current_id
                };

                if let Some(id) = channel_id {
                    // Fetch and update program information
                    if let Some(new_title) =
                        update_program_info(&ui_weak, &api_client_clone, id).await
                    {
                        // Check if the program has changed
                        if last_program_title.as_ref() != Some(&new_title) {
                            println!("Program changed to: {}", new_title);
                            last_program_title = Some(new_title);
                        }
                    }
                } else {
                    // No channel playing, reset last program title
                    last_program_title = None;
                }
            }
        });
    }

    println!("Periodic program update task started");

    // ========================================================================
    // STEP 5: RUN THE APPLICATION
    // ========================================================================

    println!("Starting UI event loop...");
    println!("\nSR Player is running!");
    println!("Select a channel from the list to start playing.\n");

    // Run the UI event loop (like app.exec() in Qt or root.mainloop() in Tkinter)
    // This blocks until the window is closed
    ui.run()?;

    println!("Application closed");

    // Return Ok(()) to indicate success
    // Like: return 0 in C or sys.exit(0) in Python
    Ok(())
}
