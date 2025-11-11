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

// MODULE DECLARATIONS
// Like 'import' in Python or JavaScript
// These tell Rust where to find our code modules
mod core; // This loads src/core/mod.rs

// Use statements bring items into scope
// Like: from core.api import SrApiClient in Python
// Or: import { SrApiClient } from './core/api' in JavaScript
use core::api::SrApiClient;
use core::episode_cache::EpisodeCache;
use core::file_player_send_safe::SendSafeFilePlayer;
use core::gapless_send_safe::SendSafeGaplessPlayer; // M3: Send-safe gapless streaming!
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

/// Macro for starting audio playback with UI updates
macro_rules! start_playback {
    ($player:expr, $url:expr, $ui_weak:expr) => {{
        let player = $player.clone();
        let ui_weak = $ui_weak.clone();
        let url = $url;

        async move {
            match player.start_stream(url).await {
                Ok(_) => {
                    let _ = ui_weak.upgrade_in_event_loop(|ui| {
                        ui.set_is_playing(true);
                        ui.set_is_loading(false);
                    });
                }
                Err(e) => {
                    eprintln!("Failed to start stream: {}", e);
                    let _ = ui_weak.upgrade_in_event_loop(|ui| {
                        ui.set_is_playing(false);
                        ui.set_is_loading(false);
                    });
                }
            }
        }
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
    env_logger::init();

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

    // Create the Send-safe gapless streaming player (M3!)
    // Uses dedicated audio thread to handle non-Send OutputStream
    // Provides truly gapless playback with continuous AAC decoding via Symphonia
    let streaming_player = Arc::new(SendSafeGaplessPlayer::new()?);

    // Create episode cache for background downloads
    let episode_cache = EpisodeCache::new();

    // Create file player for seekable playback (when episode is downloaded)
    let file_player = Arc::new(SendSafeFilePlayer::new()?);

    // Track current episode URL for seeking
    let current_episode_url: Arc<std::sync::Mutex<Option<String>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Track which player is active (true = file_player, false = streaming_player)
    let using_file_player: Arc<std::sync::Mutex<bool>> = Arc::new(std::sync::Mutex::new(false));

    // Initialize volume to match UI default (0.5 = 50%)
    // Note: UI uses logarithmic scale, so 0.5 slider = 0.25 actual volume
    streaming_player.set_volume(0.25);
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

    // CALLBACK 1: When user selects a channel
    {
        let ui_weak = ui.as_weak();
        let player = streaming_player.clone();
        let file_player_clone = file_player.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();
        let api_client_clone = api_client.clone();
        let channel_map_clone = channel_map.clone();
        let current_channel_id_clone = current_channel_id.clone();

        ui.on_channel_selected(move |channel_id, stream_url| {
            println!("Channel selected: ID={}, URL={}", channel_id, stream_url);

            // Stop file player if it's playing an episode
            file_player_clone.stop();
            if let Ok(mut using_file) = using_file_player_clone.lock() {
                *using_file = false;
            }

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
            let player_clone = player.clone();
            let runtime_clone = runtime_handle.clone();

            // Spawn async task for program info and gapless streaming
            runtime_handle.spawn(async move {
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

                runtime_clone.spawn(start_playback!(player_clone, stream_url, ui_weak_clone));
            });
        });
    }

    // CALLBACK 2: Play/Pause button
    {
        let ui_weak = ui.as_weak();
        let streaming_player_clone = streaming_player.clone();
        let file_player_clone = file_player.clone();
        let using_file_player_clone = using_file_player.clone();

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
                    streaming_player_clone.pause();
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
                        streaming_player_clone.resume();
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
        let streaming_player_clone = streaming_player.clone();
        let file_player_clone = file_player.clone();

        ui.on_volume_changed(move |volume| {
            streaming_player_clone.set_volume(volume);
            file_player_clone.set_volume(volume);
        });
    }

    // CALLBACK 2c: Seek to position (works once episode is downloaded)
    {
        let ui_weak = ui.as_weak();
        let episode_cache_clone = episode_cache.clone();
        let current_episode_url_clone = current_episode_url.clone();
        let file_player_clone = file_player.clone();
        let streaming_player_clone = streaming_player.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();

        ui.on_seek_to_position(move |position| {
            println!("Seeking to position: {:.1}s", position);

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
            let using_file_clone = using_file_player_clone.clone();
            let ui_clone = ui_weak.clone();

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

                // Stop both players immediately before seeking
                streaming_clone.stop().await;
                player_clone.stop();

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
                        let _ = ui_clone.upgrade_in_event_loop(move |ui| {
                            ui.set_playback_position(position);
                            ui.set_is_playing(true);
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to seek: {}", e);

                        // Resume streaming as fallback
                        streaming_clone.resume();
                    }
                }
            });
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
            let mut position = (current_minutes - start_minutes) as f32 * 60.0; // seconds

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
        filter = |programs, groups| core::podcast::programs_to_items_news(programs, groups),
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
        let streaming_player_clone = streaming_player.clone();
        let file_player_clone = file_player.clone();
        let episode_cache_clone = episode_cache.clone();
        let current_episode_url_clone = current_episode_url.clone();
        let using_file_player_clone = using_file_player.clone();
        let runtime_handle = runtime.handle().clone();
        let all_programs_clone = all_programs.clone();
        let current_channel_id_clone = current_channel_id.clone();

        ui.on_episode_selected(move |episode_id| {
            // Stop both players before starting new episode
            let streaming_clone = streaming_player_clone.clone();
            let file_clone = file_player_clone.clone();
            let using_file_clone = using_file_player_clone.clone();
            let runtime = runtime_handle.clone();

            runtime.spawn(async move {
                streaming_clone.stop().await;
                file_clone.stop();

                // Reset player tracking flag
                if let Ok(mut using_file) = using_file_clone.lock() {
                    *using_file = false;
                }
            });

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

            // Start playback using streaming player (fast loading)
            let url = episode.url.to_string();

            // Store current episode URL
            if let Ok(mut current_url) = current_episode_url_clone.lock() {
                *current_url = Some(url.clone());
            }

            // Start background download for seeking with progress tracking
            let ui_clone_for_progress = ui_weak.clone();
            episode_cache_clone.start_download(url.clone(), move |downloaded, total| {
                let percent = if total > 0 {
                    (downloaded as f64 / total as f64 * 100.0) as i32
                } else {
                    0
                };

                let ui_for_clear = ui_clone_for_progress.clone();
                let _ = ui_clone_for_progress.upgrade_in_event_loop(move |ui| {
                    if downloaded >= total && total > 0 {
                        ui.set_download_status("Downloaded (100%)".into());
                        // Clear after 2 seconds
                        slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                            let _ = ui_for_clear.upgrade_in_event_loop(|ui| {
                                ui.set_download_status("".into());
                            });
                        });
                    } else {
                        ui.set_download_status(format!("Downloading ({}%)", percent).into());
                    }
                });
            });

            runtime_handle.spawn(start_playback!(streaming_player_clone, url, ui_weak));
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

    println!("UI callbacks configured");

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
