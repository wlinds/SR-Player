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
use core::gapless_send_safe::SendSafeGaplessPlayer; // M3: Send-safe gapless streaming!
use core::utils::{bytes_to_slint_image, download_image_bytes, parse_sr_date_to_time};

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

/// Create an empty ProgramInfo struct (helper to avoid duplication)
fn empty_program_info() -> ProgramInfo {
    ProgramInfo {
        title: "".into(),
        description: "".into(),
        start_time: "".into(),
        end_time: "".into(),
        show_image: slint::Image::default(),
        program_id: 0,
        has_podcasts: false,
    }
}

/// Fetch and update program information for a given channel ID
/// Returns the current program title to detect changes
async fn update_program_info(
    ui_weak: &slint::Weak<MainWindow>,
    api_client: &SrApiClient,
    channel_id: i32,
) -> Option<String> {
    // Fetch current program information in a blocking task
    let api_clone = api_client.clone();
    let program_info =
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
        .flatten();

    // Update UI with program information
    if let Some(episode) = program_info {
        let program_title = episode.title.clone();

        // Download image bytes in blocking task
        let image_url = episode.social_image.clone().unwrap_or_default();
        let image_bytes = tokio::task::spawn_blocking(move || download_image_bytes(&image_url))
            .await
            .ok()
            .flatten();

        ui_weak
            .upgrade_in_event_loop(move |ui| {
                // Format times (convert from /Date(ms)/ format to HH:MM)
                let start_time = parse_sr_date_to_time(&episode.start_time_utc);
                let end_time = parse_sr_date_to_time(&episode.end_time_utc);

                // Convert bytes to Slint Image on main thread (Image is not Send)
                let show_image = image_bytes
                    .and_then(bytes_to_slint_image)
                    .unwrap_or_default();

                // Get program ID
                let program_id = episode.program.as_ref().map(|p| p.id).unwrap_or(0);

                ui.set_current_program(ProgramInfo {
                    title: episode.title.clone().into(),
                    description: episode.description.unwrap_or_default().into(),
                    start_time: start_time.into(),
                    end_time: end_time.into(),
                    show_image,
                    program_id: program_id as i32,
                    has_podcasts: false,
                });
            })
            .ok();

        Some(program_title)
    } else {
        None
    }
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

    println!("Backend components initialized");

    // ========================================================================
    // STEP 3: FETCH AND DISPLAY CHANNELS
    // ========================================================================

    println!("Fetching channels from Sveriges Radio API...");

    // Fetch channels from the API
    let channels = api_client.get_channels()?;

    println!("Fetched {} channels", channels.len());

    // Create HashMap for O(1) channel name lookups (avoids O(n) search on each selection)
    // Note: We use i32 keys because Slint only supports 'int' (i32) in .slint files
    let channel_map: HashMap<i32, String> = channels
        .iter()
        .filter_map(|ch| {
            // Validate channel ID fits in i32 range (very unlikely to fail in practice)
            i32::try_from(ch.id).ok().map(|id| (id, ch.name.clone()))
        })
        .collect();

    // Convert API response to UI model
    // We need to transform our Rust structs into Slint's ChannelItem struct
    let channel_items: Vec<ChannelItem> = channels
        .iter() // Iterate over channels (like for channel in channels)
        .filter_map(|channel| {
            // For each channel, create a ChannelItem
            // Use try_from for safe u32->i32 conversion (Slint limitation)
            i32::try_from(channel.id).ok().map(|id| ChannelItem {
                id,
                name: channel.name.clone().into(),
                stream_url: channel.live_audio.url.clone().into(),
            })
        })
        .collect(); // Collect into a Vec<ChannelItem>

    // Create a Slint model from the Vec
    // ModelRc is like a reactive array in Vue or state array in React
    let channels_model = Rc::new(VecModel::from(channel_items));

    // Set the channels in the UI
    ui.set_channels(ModelRc::from(channels_model));

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

            let ui = ui_weak.upgrade().unwrap();
            ui.set_is_loading(true);

            // Get the channel name using O(1) HashMap lookup
            let channel_name = channel_map_clone
                .get(&channel_id)
                .cloned()
                .unwrap_or_else(|| String::from("Unknown"));
            ui.set_current_channel_name(channel_name.into());

            let stream_url = stream_url.to_string();
            let ui_clone = ui_weak.clone();
            let player_clone = player.clone();
            let api_clone = api_client_clone.clone();

            // M3: Spawn async task for gapless streaming
            runtime_handle.spawn(async move {
                // Fetch current program information in a blocking task
                let api_clone_for_blocking = api_clone.clone();
                let program_info = tokio::task::spawn_blocking(move || {
                    match api_clone_for_blocking.get_schedule_right_now() {
                        Ok(schedule) => {
                            // Find the current channel's schedule
                            // Safe conversion: channel_id is guaranteed valid (came from UI)
                            schedule
                                .channels
                                .iter()
                                .find(|ch| ch.id == channel_id as u32)
                                .and_then(|ch| ch.current_episode.clone())
                        }
                        Err(e) => {
                            eprintln!("Failed to fetch schedule: {}", e);
                            None
                        }
                    }
                })
                .await
                .ok()
                .flatten();

                // Update UI with program information
                if let Some(episode) = program_info {
                    // Download image bytes in blocking task
                    let image_url = episode.social_image.clone().unwrap_or_default();
                    let image_bytes =
                        tokio::task::spawn_blocking(move || download_image_bytes(&image_url))
                            .await
                            .ok()
                            .flatten();

                    ui_clone
                        .upgrade_in_event_loop(move |ui| {
                            // Format times (convert from /Date(ms)/ format to HH:MM)
                            let start_time = parse_sr_date_to_time(&episode.start_time_utc);
                            let end_time = parse_sr_date_to_time(&episode.end_time_utc);

                            // Convert bytes to Slint Image on main thread (Image is not Send)
                            let show_image = image_bytes
                                .and_then(bytes_to_slint_image)
                                .unwrap_or_default();

                            // Get program ID and check if it has podcasts
                            let program_id = episode.program.as_ref().map(|p| p.id).unwrap_or(0);

                            ui.set_current_program(ProgramInfo {
                                title: episode.title.clone().into(),
                                description: episode.description.unwrap_or_default().into(),
                                start_time: start_time.into(),
                                end_time: end_time.into(),
                                show_image,
                                program_id: program_id as i32,
                                has_podcasts: false, // We'll check this later if needed
                            });
                        })
                        .ok();
                }

                match player_clone.start_stream(stream_url).await {
                    Ok(_) => {
                        println!("Gapless streaming started");
                        if let Err(e) = ui_clone.upgrade_in_event_loop(|ui| {
                            ui.set_is_playing(true);
                            ui.set_is_loading(false);
                        }) {
                            eprintln!("Failed to update UI after stream start: {:?}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to start stream: {}", e);
                        if let Err(e) = ui_clone.upgrade_in_event_loop(|ui| {
                            ui.set_is_playing(false);
                            ui.set_is_loading(false);
                        }) {
                            eprintln!("Failed to update UI after stream error: {:?}", e);
                        }
                    }
                }
            });
        });
    }

    // CALLBACK 2: Stop button
    {
        let ui_weak = ui.as_weak();
        let player = streaming_player.clone();
        let runtime_handle = runtime.handle().clone();
        let current_channel_id_clone = current_channel_id.clone();

        ui.on_stop_clicked(move || {
            println!("Stopping playback");

            // Clear the current channel ID to stop periodic program updates
            if let Ok(mut current_id) = current_channel_id_clone.lock() {
                *current_id = None;
            }

            let ui_clone = ui_weak.clone();
            let player_clone = player.clone();

            // stop() is async, spawn task
            runtime_handle.spawn(async move {
                player_clone.stop().await;
                if let Err(e) = ui_clone.upgrade_in_event_loop(|ui| {
                    ui.set_is_playing(false);
                    ui.set_current_channel_name("No channel selected".into());
                    ui.set_current_program(empty_program_info());
                }) {
                    eprintln!("Failed to update UI after stop: {:?}", e);
                }
            });
        });
    }

    // CALLBACK 3: Podcasts tab clicked - lazy load programs with grouping
    {
        let ui_weak = ui.as_weak();
        let runtime_handle = runtime.handle().clone();
        let api_client_clone = api_client.clone();
        let all_programs_clone = all_programs.clone();

        ui.on_podcasts_tab_clicked(move || {
            let ui = ui_weak.upgrade().unwrap();

            // Check if we have data already loaded
            let programs_available = {
                let all_programs_lock = all_programs_clone.lock().unwrap();
                !all_programs_lock.is_empty()
            };

            if programs_available {
                // Filter existing data for Podcasts tab
                let all_programs_lock = all_programs_clone.lock().unwrap();
                let program_items = core::podcast::programs_to_items_podcasts(&all_programs_lock);
                let programs_model = Rc::new(VecModel::from(program_items));
                ui.set_programs(ModelRc::from(programs_model));
            } else {
                // First time loading - fetch data
                println!("Loading programs with podcasts...");

                let ui_clone = ui_weak.clone();
                let api_clone = api_client_clone.clone();
                let all_programs_clone2 = all_programs_clone.clone();

                runtime_handle.spawn(async move {
                    match core::podcast::fetch_programs_with_podcasts(&api_clone).await {
                        Ok(programs) => {
                            // Store programs for future use by all tabs
                            {
                                let mut all_programs_lock = all_programs_clone2.lock().unwrap();
                                *all_programs_lock = programs.clone();
                            }

                            // Show filtered data for Podcasts tab
                            if let Err(e) = ui_clone.upgrade_in_event_loop(move |ui| {
                                let program_items =
                                    core::podcast::programs_to_items_podcasts(&programs);
                                let programs_model = Rc::new(VecModel::from(program_items));
                                ui.set_programs(ModelRc::from(programs_model));
                                ui.set_programs_loaded(true);
                            }) {
                                eprintln!("Failed to update UI with programs: {:?}", e);
                            }
                        }
                        Err(e) => eprintln!("Failed to fetch programs: {}", e),
                    }
                });
            }
        });

        // News tab callback - reuse the same logic as podcasts but for news filtering
        let ui_weak_news = ui.as_weak();
        let api_client_clone_news = api_client.clone();
        let groups_expanded_clone_news = groups_expanded.clone();
        let all_programs_clone_news = all_programs.clone();
        let runtime_handle_news = runtime.handle().clone();

        ui.on_news_tab_clicked(move || {
            let ui = ui_weak_news.upgrade().unwrap();

            // Check if we have data already loaded
            let programs_available = {
                let all_programs_lock = all_programs_clone_news.lock().unwrap();
                !all_programs_lock.is_empty()
            };

            if programs_available {
                // Filter existing data for News tab
                let all_programs_lock = all_programs_clone_news.lock().unwrap();
                let groups_expanded_lock = groups_expanded_clone_news.lock().unwrap();
                let program_items = core::podcast::programs_to_items_news(
                    &all_programs_lock,
                    &groups_expanded_lock,
                );
                let programs_model = Rc::new(VecModel::from(program_items));
                ui.set_news_programs(ModelRc::from(programs_model));
            } else {
                // First time loading - fetch data
                println!("Loading programs for news tab...");

                let ui_clone = ui_weak_news.clone();
                let api_clone = api_client_clone_news.clone();
                let groups_expanded_clone2 = groups_expanded_clone_news.clone();
                let all_programs_clone2 = all_programs_clone_news.clone();

                runtime_handle_news.spawn(async move {
                    match core::podcast::fetch_programs_with_podcasts(&api_clone).await {
                        Ok(programs) => {
                            // Store programs for future use by all tabs
                            {
                                let mut all_programs_lock = all_programs_clone2.lock().unwrap();
                                *all_programs_lock = programs.clone();
                            }

                            // Show filtered data for News tab
                            if let Err(e) = ui_clone.upgrade_in_event_loop(move |ui| {
                                let groups_expanded_lock = groups_expanded_clone2.lock().unwrap();
                                let program_items = core::podcast::programs_to_items_news(
                                    &programs,
                                    &groups_expanded_lock,
                                );
                                let programs_model = Rc::new(VecModel::from(program_items));
                                ui.set_news_programs(ModelRc::from(programs_model));
                                ui.set_news_loaded(true);
                            }) {
                                eprintln!("Failed to update UI with programs: {:?}", e);
                            }
                        }
                        Err(e) => eprintln!("Failed to fetch programs: {}", e),
                    }
                });
            }
        });
    }

    // CALLBACK 4: Browse podcasts button (M4)
    {
        let ui_weak = ui.as_weak();
        let runtime_handle = runtime.handle().clone();
        let api_client_clone = api_client.clone();

        ui.on_browse_podcasts_clicked(move |program_id| {
            println!("Browse podcasts clicked for program ID: {}", program_id);

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
                            ui.set_current_tab(1); // Switch to podcasts tab
                            ui.set_show_episodes_view(true);
                        }) {
                            eprintln!("Failed to update UI with episodes: {:?}", e);
                        }
                    }
                    Err(e) => eprintln!("Failed to fetch episodes: {}", e),
                }
            });
        });
    }

    // CALLBACK 5: Program selected in podcasts tab (M4)
    // Fetches and displays episodes for the selected program
    {
        let ui_weak = ui.as_weak();
        let runtime_handle = runtime.handle().clone();
        let api_client_clone = api_client.clone();

        ui.on_program_selected(move |program_id| {
            println!("Program selected: ID={}", program_id);

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
                        }) {
                            eprintln!("Failed to update UI with episodes: {:?}", e);
                        }
                    }
                    Err(e) => eprintln!("Failed to fetch episodes: {}", e),
                }
            });
        });
    }

    // CALLBACK 6: Episode selected for playback (M4)
    {
        let ui_weak = ui.as_weak();
        let player = streaming_player.clone();
        let runtime_handle = runtime.handle().clone();
        let all_programs_clone = all_programs.clone();
        let current_channel_id_clone = current_channel_id.clone();

        ui.on_episode_selected(move |episode_id| {
            println!("Episode selected: ID={}", episode_id);

            // Clear the current channel ID to stop live program polling
            if let Ok(mut current_id) = current_channel_id_clone.lock() {
                *current_id = None;
            }

            let ui = ui_weak.upgrade().unwrap();
            ui.set_is_loading(true);
            ui.set_current_channel_name("Podcast".into());

            // Find the selected episode in the episodes list
            let episodes = ui.get_episodes();
            let mut selected_episode: Option<EpisodeItem> = None;
            for i in 0..episodes.row_count() {
                if let Some(episode) = episodes.row_data(i) {
                    if episode.id == episode_id {
                        selected_episode = Some(episode);
                        break;
                    }
                }
            }

            if let Some(episode) = selected_episode {
                // Find the program image from cached programs using the selected program ID
                let current_program_id = ui.get_selected_program_id();
                let program_image_url = {
                    let all_programs_lock = all_programs_clone.lock().unwrap();
                    all_programs_lock
                        .iter()
                        .find(|p| p.id as i32 == current_program_id)
                        .and_then(|p| p.program_image.clone().or(p.social_image.clone()))
                };

                // Set episode information in the top UI (initially without image)
                ui.set_current_program(ProgramInfo {
                    title: episode.title.clone(),
                    description: episode.description.clone(),
                    start_time: "".into(), // Not applicable for podcasts
                    end_time: episode.publish_date.clone(), // Show publish date instead
                    show_image: slint::Image::default(), // Will be updated below if image is available
                    program_id: 0,                       // Not applicable for episodes
                    has_podcasts: false,
                });

                let episode_url = episode.url.to_string();
                let ui_clone = ui_weak.clone();
                let player_clone = player.clone();

                // Load program image in background if available
                if let Some(image_url) = program_image_url {
                    let ui_clone_for_image = ui_weak.clone();
                    let episode_title = episode.title.clone();
                    let episode_description = episode.description.clone();
                    let episode_date = episode.publish_date.clone();

                    runtime_handle.spawn(async move {
                        // Download image bytes in blocking task
                        let image_bytes =
                            tokio::task::spawn_blocking(move || download_image_bytes(&image_url))
                                .await
                                .ok()
                                .flatten();

                        // Update UI with image on main thread
                        let _ = ui_clone_for_image.upgrade_in_event_loop(move |ui| {
                            // Convert bytes to Slint Image on main thread (Image is not Send)
                            let show_image = image_bytes
                                .and_then(bytes_to_slint_image)
                                .unwrap_or_default();

                            ui.set_current_program(ProgramInfo {
                                title: episode_title,
                                description: episode_description,
                                start_time: "".into(),
                                end_time: episode_date,
                                show_image,
                                program_id: 0,
                                has_podcasts: false,
                            });
                        });
                    });
                }

                runtime_handle.spawn(async move {
                    match player_clone.start_stream(episode_url).await {
                        Ok(_) => {
                            println!("Podcast playback started");
                            if let Err(e) = ui_clone.upgrade_in_event_loop(|ui| {
                                ui.set_is_playing(true);
                                ui.set_is_loading(false);
                            }) {
                                eprintln!("Failed to update UI after podcast start: {:?}", e);
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to start podcast: {}", e);
                            if let Err(e) = ui_clone.upgrade_in_event_loop(|ui| {
                                ui.set_is_playing(false);
                                ui.set_is_loading(false);
                            }) {
                                eprintln!("Failed to update UI after podcast error: {:?}", e);
                            }
                        }
                    }
                });
            } else {
                eprintln!("Episode with ID {} not found", episode_id);
                let ui_clone = ui_weak.clone();
                if let Err(e) = ui_clone.upgrade_in_event_loop(|ui| {
                    ui.set_is_loading(false);
                }) {
                    eprintln!("Failed to update UI after episode not found: {:?}", e);
                }
            }
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

            let ui = ui_weak.upgrade().unwrap();

            // Toggle group expansion state
            {
                let mut groups_expanded_lock = groups_expanded_clone.lock().unwrap();
                let current_state = groups_expanded_lock
                    .get(&group_id)
                    .copied()
                    .unwrap_or(false);
                groups_expanded_lock.insert(group_id, !current_state);
            }

            // Refresh program list to show/hide children based on current tab
            {
                let all_programs_lock = all_programs_clone.lock().unwrap();
                let groups_expanded_lock = groups_expanded_clone.lock().unwrap();
                let current_tab = ui.get_current_tab();

                if current_tab == 2 {
                    // News tab - show only news programs
                    let program_items = core::podcast::programs_to_items_news(
                        &all_programs_lock,
                        &groups_expanded_lock,
                    );
                    let programs_model = Rc::new(VecModel::from(program_items));
                    ui.set_news_programs(ModelRc::from(programs_model));
                } else if current_tab == 1 {
                    // Podcasts tab - exclude news programs
                    let program_items =
                        core::podcast::programs_to_items_podcasts(&all_programs_lock);
                    let programs_model = Rc::new(VecModel::from(program_items));
                    ui.set_programs(ModelRc::from(programs_model));
                }
            }
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
