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

/// Set the macOS process name displayed in the menu bar.
/// When running via `cargo run`, macOS uses the binary name ("sr-player").
/// This overrides it to show "SR Player" instead.
#[cfg(target_os = "macos")]
fn set_macos_app_name() {
    use std::ffi::{c_char, c_void, CString};

    type Id = *mut c_void;
    type Sel = *mut c_void;

    // Non-variadic function pointer types for objc_msgSend casts.
    // On ARM64, objc_msgSend uses register-based calling conventions that
    // differ from C variadic ABI, so we must cast to exact signatures.
    type MsgSendNoArgs = unsafe extern "C" fn(Id, Sel) -> Id;
    type MsgSendOnePtr = unsafe extern "C" fn(Id, Sel, *const c_char) -> Id;
    type MsgSendSetObj = unsafe extern "C" fn(Id, Sel, Id);

    extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    unsafe {
        let send_no_args: MsgSendNoArgs = std::mem::transmute(objc_msgSend as *const c_void);
        let send_one_ptr: MsgSendOnePtr = std::mem::transmute(objc_msgSend as *const c_void);
        let send_set_obj: MsgSendSetObj = std::mem::transmute(objc_msgSend as *const c_void);

        let cls = objc_getClass(c"NSProcessInfo".as_ptr());
        if cls.is_null() {
            return;
        }

        let process_info_sel = sel_registerName(c"processInfo".as_ptr());
        let process_info = send_no_args(cls, process_info_sel);
        if process_info.is_null() {
            return;
        }

        // Create NSString with "SR Player"
        let nsstring_cls = objc_getClass(c"NSString".as_ptr());
        let alloc_sel = sel_registerName(c"alloc".as_ptr());
        let init_sel = sel_registerName(c"initWithUTF8String:".as_ptr());
        let name_cstr = CString::new("SR Player").unwrap();

        let nsstring = send_no_args(nsstring_cls, alloc_sel);
        let nsstring = send_one_ptr(nsstring, init_sel, name_cstr.as_ptr());

        // Set the process name
        let set_name_sel = sel_registerName(c"setProcessName:".as_ptr());
        send_set_obj(process_info, set_name_sel, nsstring);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_macos_app_name() {
    // No-op on other platforms
}

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
    // Set macOS process name before anything else
    set_macos_app_name();

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

    // Set initial keep-channels-alive from saved settings
    ui.set_keep_channels_alive(app_state.get_keep_channels_alive());

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
