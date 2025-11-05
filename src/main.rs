// SR Player - Main entry point
//
// This is where the program starts (like if __name__ == "__main__" in Python)
//
// PROGRAM FLOW:
// 1. Initialize logging (for debugging)
// 2. Load Slint UI
// 3. Create API client and audio player
// 4. Fetch channels from SR API
// 5. Set up UI callbacks (event handlers)
// 6. Show window and run event loop

// MODULE DECLARATIONS
// Like 'import' in Python or JavaScript
// These tell Rust where to find our code modules
mod core; // This loads src/core/mod.rs

// Use statements bring items into scope
// Like: from core.api import SrApiClient in Python
// Or: import { SrApiClient } from './core/api' in JavaScript
use core::api::SrApiClient;
use core::gapless_send_safe::SendSafeGaplessPlayer; // M3: Send-safe gapless streaming!

// Import Slint types
use slint::{ModelRc, VecModel};

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

/// Parse SR API date format /Date(milliseconds)/ and convert to HH:MM format
/// Example: "/Date(1762431000000)/" -> "10:30"
fn parse_sr_date_to_time(date_str: &str) -> String {
    use chrono::{DateTime, Utc};

    if let Some(ms_str) = date_str
        .strip_prefix("/Date(")
        .and_then(|s| s.strip_suffix(")/"))
    {
        if let Ok(ms) = ms_str.parse::<i64>() {
            return DateTime::from_timestamp_millis(ms)
                .map(|dt: DateTime<Utc>| dt.format("%H:%M").to_string())
                .unwrap_or_else(|| String::from("--:--"));
        }
    }
    String::from("--:--")
}

/// Create an empty ProgramInfo struct (helper to avoid duplication)
fn empty_program_info() -> ProgramInfo {
    ProgramInfo {
        title: "".into(),
        description: "".into(),
        start_time: "".into(),
        end_time: "".into(),
        show_image: slint::Image::default(),
    }
}

/// Download image bytes from URL
fn download_image_bytes(url: &str) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }

    match reqwest::blocking::get(url) {
        Ok(response) => {
            response.bytes().ok().map(|b| b.to_vec())
        }
        Err(e) => {
            eprintln!("Failed to download image from {}: {}", url, e);
            None
        }
    }
}

/// Convert image bytes to Slint Image using the image crate
fn bytes_to_slint_image(bytes: Vec<u8>) -> Option<slint::Image> {
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = (rgba.width(), rgba.height());
            let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                rgba.as_raw(),
                width,
                height,
            );
            Some(slint::Image::from_rgba8(buffer))
        }
        Err(e) => {
            eprintln!("Failed to decode image: {}", e);
            None
        }
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

        ui.on_channel_selected(move |channel_id, stream_url| {
            println!("Channel selected: ID={}, URL={}", channel_id, stream_url);

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
                    let image_bytes = tokio::task::spawn_blocking(move || {
                        download_image_bytes(&image_url)
                    })
                    .await
                    .ok()
                    .flatten();

                    ui_clone
                        .upgrade_in_event_loop(move |ui| {
                            // Format times (convert from /Date(ms)/ format to HH:MM)
                            let start_time = parse_sr_date_to_time(&episode.start_time_utc);
                            let end_time = parse_sr_date_to_time(&episode.end_time_utc);

                            // Convert image bytes to Slint Image on the main thread
                            let show_image = image_bytes
                                .and_then(bytes_to_slint_image)
                                .unwrap_or_default();

                            ui.set_current_program(ProgramInfo {
                                title: episode.title.clone().into(),
                                description: episode.description.unwrap_or_default().into(),
                                start_time: start_time.into(),
                                end_time: end_time.into(),
                                show_image,
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

        ui.on_stop_clicked(move || {
            println!("Stopping playback");

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

    println!("UI callbacks configured");

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
