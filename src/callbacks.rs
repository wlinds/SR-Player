// UI Callback Handlers
//
// This module sets up all UI event handlers (callbacks) for the application.
// Each callback handles a specific user interaction like clicking a button,
// selecting a channel, or toggling a favorite.

use crate::app_state::AppState;
use crate::core;
use crate::core::api::SrApiClient;
use crate::core::models::ActivePlayer;
use crate::core::utils::{bytes_to_slint_image, calculate_program_position, fetch_image_bytes};
use crate::{ChannelItem, EpisodeItem, MainWindow, ProgramInfo, ProgramItem};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::collections::HashMap;
use std::rc::Rc;

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Helper to fetch program info from schedule for a channel
pub async fn fetch_program_info_for_channel(
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
pub fn program_info_from_episode(
    episode: &core::models::ScheduledEpisode,
    show_image: slint::Image,
) -> ProgramInfo {
    let start_time = core::utils::parse_sr_date_to_time(&episode.start_time_utc);
    let end_time = core::utils::parse_sr_date_to_time(&episode.end_time_utc);
    let program_id = episode.program.as_ref().map(|p| p.id).unwrap_or(0);

    // Build episode URL for sharing (if episode_id is available)
    let episode_url = episode
        .episode_id
        .map(|id| format!("https://sverigesradio.se/avsnitt/{}", id))
        .unwrap_or_default();

    ProgramInfo {
        title: episode.title.clone().into(),
        description: episode.description.clone().unwrap_or_default().into(),
        start_time: start_time.into(),
        end_time: end_time.into(),
        show_image,
        program_id: program_id as i32,
        has_podcasts: false,
        episode_url: episode_url.into(),
    }
}

/// Refresh program list based on current tab
fn refresh_program_list(
    ui: &MainWindow,
    all_programs: &[core::models::Program],
    groups_expanded: &HashMap<i32, bool>,
) {
    let current_tab = ui.get_current_tab();
    if current_tab == 2 {
        let items = core::podcast::programs_to_items_news(all_programs, groups_expanded);
        ui.set_news_programs(ModelRc::from(Rc::new(VecModel::from(items))));
    } else if current_tab == 1 {
        let items = core::podcast::programs_to_items_podcasts(all_programs);
        ui.set_programs(ModelRc::from(Rc::new(VecModel::from(items))));
    }
}

/// Fetch and update program information for a given channel ID
pub async fn update_program_info(
    ui_weak: &slint::Weak<MainWindow>,
    api_client: &SrApiClient,
    channel_id: i32,
) -> Option<String> {
    let episode = fetch_program_info_for_channel(api_client, channel_id).await?;
    let program_title = episode.title.clone();

    let image_url = episode.social_image.clone().unwrap_or_default();
    let image_bytes = fetch_image_bytes(image_url).await;

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
// CALLBACK SETUP
// ============================================================================

/// Set up all UI callbacks
pub fn setup_callbacks(
    ui: &MainWindow,
    app_state: &AppState,
    api_client: &SrApiClient,
    channel_map: &HashMap<i32, String>,
) {
    setup_channel_selection(ui, app_state, api_client, channel_map);
    setup_playback_controls(ui, app_state);
    setup_tab_callbacks(ui, app_state, api_client);
    setup_program_callbacks(ui, app_state, api_client);
    setup_episode_callback(ui, app_state);
    setup_favorite_callbacks(ui, app_state);
    setup_misc_callbacks(ui, app_state);
}

/// Channel selection handler
fn setup_channel_selection(
    ui: &MainWindow,
    app_state: &AppState,
    api_client: &SrApiClient,
    channel_map: &HashMap<i32, String>,
) {
    let ui_weak = ui.as_weak();
    let state = app_state.clone();
    let api_client_clone = api_client.clone();
    let channel_map_clone = channel_map.clone();

    ui.on_channel_selected(move |channel_id, stream_url| {
        println!("Channel selected: ID={}, URL={}", channel_id, stream_url);

        state.set_current_channel_id(Some(channel_id));

        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_is_loading(true);
        ui.set_playback_duration(0.0);
        ui.set_playback_position(0.0);
        ui.set_program_duration(0.0);

        let channel_name = channel_map_clone
            .get(&channel_id)
            .cloned()
            .unwrap_or_else(|| String::from("Unknown"));
        ui.set_current_channel_name(channel_name.into());

        let stream_url = stream_url.to_string();
        let ui_weak_clone = ui_weak.clone();
        let api_clone = api_client_clone.clone();
        let state_inner = state.clone();

        state.spawn(async move {
            state_inner.file_player.stop();
            state_inner.streaming_player.stop().await;
            state_inner.set_active_player(ActivePlayer::ChannelPool);

            if let Some(episode) = fetch_program_info_for_channel(&api_clone, channel_id).await {
                let image_bytes =
                    fetch_image_bytes(episode.social_image.clone().unwrap_or_default()).await;
                let _ = ui_weak_clone.upgrade_in_event_loop(move |ui| {
                    let show_image = image_bytes
                        .and_then(bytes_to_slint_image)
                        .unwrap_or_default();
                    ui.set_current_program(program_info_from_episode(&episode, show_image));
                    ui.invoke_update_live_position();
                });
            }

            let start = std::time::Instant::now();
            match state_inner
                .channel_pool
                .switch_to_channel(channel_id, stream_url)
                .await
            {
                Ok(()) => {
                    println!("Channel switch completed in {}ms", start.elapsed().as_millis());
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

/// Play/pause, volume, seek, and live position callbacks
fn setup_playback_controls(ui: &MainWindow, app_state: &AppState) {
    // Play/pause
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();

        ui.on_play_pause_clicked(move || {
            let ui_clone = ui_weak.upgrade().unwrap();
            let is_playing = ui_clone.get_is_playing();
            let current_player = state.get_active_player();

            if is_playing {
                println!("Pausing {:?}", current_player);
                let state_inner = state.clone();
                state.spawn(async move { state_inner.pause_active().await });
                ui_clone.set_is_playing(false);
            } else {
                let channel_name = ui_clone.get_current_channel_name();
                if channel_name != "No channel selected" && current_player != ActivePlayer::None {
                    println!("Resuming {:?}", current_player);
                    let state_inner = state.clone();
                    state.spawn(async move { state_inner.resume_active().await });
                    ui_clone.set_is_playing(true);
                }
            }
        });
    }

    // Volume
    {
        let state = app_state.clone();
        ui.on_volume_changed(move |volume| {
            let state_for_spawn = state.clone();
            let state_for_volume = state.clone();
            state_for_spawn.spawn(async move {
                state_for_volume.set_volume(volume).await;
            });
        });
    }

    // Seek
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();

        ui.on_seek_to_position(move |position| {
            let Some(ui_handle) = ui_weak.upgrade() else { return };

            let ctx = core::seek_handler::SeekContext::from_ui(&ui_handle, position, &state);
            let state_inner = state.clone();
            let ui_weak_clone = ui_weak.clone();

            if ctx.is_episode() {
                state.spawn(async move {
                    core::seek_handler::seek_episode(&ctx, &state_inner, &ui_weak_clone).await;
                });
            } else if ctx.is_live_radio() {
                state.spawn(async move {
                    core::seek_handler::seek_live_radio(&ctx, &state_inner, &ui_weak_clone).await;
                });
            }
        });
    }

    // Live position update
    {
        let ui_weak = ui.as_weak();
        ui.on_update_live_position(move || {
            let Some(ui) = ui_weak.upgrade() else { return };

            if ui.get_playback_duration() > 0.0 {
                return;
            }

            let program = ui.get_current_program();
            if program.start_time.is_empty() || program.end_time.is_empty() {
                return;
            }

            let Some((position, duration)) =
                calculate_program_position(&program.start_time, &program.end_time)
            else {
                return;
            };

            let position = position.max(0.0).min(duration);
            ui.set_program_duration(duration);
            ui.set_playback_position(position);
        });
    }
}

/// Tab click callbacks (Podcasts, News, Favorites)
fn setup_tab_callbacks(ui: &MainWindow, app_state: &AppState, api_client: &SrApiClient) {
    // Podcasts tab
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();
        let api_clone = api_client.clone();

        ui.on_podcasts_tab_clicked(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let has_cached = { !state.all_programs.lock().unwrap().is_empty() };

            if has_cached {
                let programs = state.all_programs.lock().unwrap();
                let favorites_lock = state.favorites.lock().unwrap();
                let mut items = core::podcast::programs_to_items_podcasts(&programs);
                for item in &mut items {
                    item.is_favorite = favorites_lock.is_program_favorite(item.id);
                }
                ui.set_programs(ModelRc::from(Rc::new(VecModel::from(items))));
            } else {
                let ui_clone = ui_weak.clone();
                let state_inner = state.clone();
                let api = api_clone.clone();
                state.spawn(async move {
                    if let Ok(programs) = core::podcast::fetch_programs_with_podcasts(&api).await {
                        *state_inner.all_programs.lock().unwrap() = programs.clone();
                        let _ = ui_clone.upgrade_in_event_loop(move |ui| {
                            let favorites_lock = state_inner.favorites.lock().unwrap();
                            let mut items = core::podcast::programs_to_items_podcasts(&programs);
                            for item in &mut items {
                                item.is_favorite = favorites_lock.is_program_favorite(item.id);
                            }
                            ui.set_programs(ModelRc::from(Rc::new(VecModel::from(items))));
                            ui.set_programs_loaded(true);
                        });
                    }
                });
            }
        });
    }

    // News tab
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();
        let api_clone = api_client.clone();

        ui.on_news_tab_clicked(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let has_cached = { !state.all_programs.lock().unwrap().is_empty() };

            if has_cached {
                let programs = state.all_programs.lock().unwrap();
                let groups = state.groups_expanded.lock().unwrap();
                let favorites_lock = state.favorites.lock().unwrap();
                let mut items = core::podcast::programs_to_items_news(&programs, &groups);
                for item in &mut items {
                    item.is_favorite = favorites_lock.is_program_favorite(item.id);
                }
                ui.set_news_programs(ModelRc::from(Rc::new(VecModel::from(items))));
            } else {
                let ui_clone = ui_weak.clone();
                let state_inner = state.clone();
                let api = api_clone.clone();
                state.spawn(async move {
                    if let Ok(programs) = core::podcast::fetch_programs_with_podcasts(&api).await {
                        *state_inner.all_programs.lock().unwrap() = programs.clone();
                        let _ = ui_clone.upgrade_in_event_loop(move |ui| {
                            let groups = state_inner.groups_expanded.lock().unwrap();
                            let favorites_lock = state_inner.favorites.lock().unwrap();
                            let mut items =
                                core::podcast::programs_to_items_news(&programs, &groups);
                            for item in &mut items {
                                item.is_favorite = favorites_lock.is_program_favorite(item.id);
                            }
                            ui.set_news_programs(ModelRc::from(Rc::new(VecModel::from(items))));
                            ui.set_news_loaded(true);
                        });
                    }
                });
            }
        });
    }

    // Favorites tab (placeholder - data already populated)
    ui.on_favorites_tab_clicked(move || {
        println!("Favorites tab clicked");
    });
}

/// Program selection and browsing callbacks
fn setup_program_callbacks(ui: &MainWindow, app_state: &AppState, api_client: &SrApiClient) {
    // Browse podcasts (from Live tab)
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();
        let api_clone = api_client.clone();

        ui.on_browse_podcasts_clicked(move |program_id| {
            let ui_clone = ui_weak.clone();
            let api = api_clone.clone();
            state.spawn(async move {
                if let Ok((episodes, name)) =
                    core::podcast::fetch_episodes_for_program(&api, program_id as u32).await
                {
                    let _ = ui_clone.upgrade_in_event_loop(move |ui| {
                        ui.set_episodes(ModelRc::from(Rc::new(VecModel::from(episodes))));
                        ui.set_selected_program_name(name.into());
                        ui.set_selected_program_id(program_id);
                        ui.set_show_episodes_view(true);
                        ui.set_current_tab(1);
                    });
                }
            });
        });
    }

    // Program selected
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();
        let api_clone = api_client.clone();

        ui.on_program_selected(move |program_id| {
            let ui_clone = ui_weak.clone();
            let api = api_clone.clone();
            state.spawn(async move {
                if let Ok((episodes, name)) =
                    core::podcast::fetch_episodes_for_program(&api, program_id as u32).await
                {
                    let _ = ui_clone.upgrade_in_event_loop(move |ui| {
                        ui.set_episodes(ModelRc::from(Rc::new(VecModel::from(episodes))));
                        ui.set_selected_program_name(name.into());
                        ui.set_selected_program_id(program_id);
                        ui.set_show_episodes_view(true);
                    });
                }
            });
        });
    }

    // Back to programs
    {
        let ui_weak = ui.as_weak();
        ui.on_back_to_programs_clicked(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_show_episodes_view(false);
            }
        });
    }

    // Group toggle (expand/collapse)
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();

        ui.on_group_toggled(move |group_id| {
            let Some(ui) = ui_weak.upgrade() else { return };

            let mut groups_lock = state.groups_expanded.lock().unwrap();
            let current = groups_lock.get(&group_id).copied().unwrap_or(false);
            groups_lock.insert(group_id, !current);

            let all_programs_lock = state.all_programs.lock().unwrap();
            refresh_program_list(&ui, &all_programs_lock, &groups_lock);
        });
    }

    // Load more (placeholder)
    ui.on_load_more_programs(|| {});

    // Search programs
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();

        ui.on_search_programs(move |search_query| {
            let Some(ui) = ui_weak.upgrade() else { return };

            let query = search_query.to_lowercase();
            let programs = state.all_programs.lock().unwrap();

            let filtered: Vec<ProgramItem> = if query.is_empty() {
                core::podcast::programs_to_items_podcasts(&programs)
            } else {
                core::podcast::programs_to_items_podcasts(&programs)
                    .into_iter()
                    .filter(|p| {
                        p.name.to_lowercase().contains(&query)
                            || p.description.to_lowercase().contains(&query)
                    })
                    .collect()
            };

            ui.set_programs(ModelRc::from(Rc::new(VecModel::from(filtered))));
        });
    }
}

/// Episode selection callback
fn setup_episode_callback(ui: &MainWindow, app_state: &AppState) {
    let ui_weak = ui.as_weak();
    let state = app_state.clone();

    ui.on_episode_selected(move |episode_id| {
        state.set_current_channel_id(None);

        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_is_loading(true);
        ui.set_current_channel_name("Podcast".into());

        let Some(episode) = find_episode_in_model(&ui.get_episodes(), episode_id) else {
            eprintln!("Episode {} not found", episode_id);
            let _ = ui_weak.upgrade_in_event_loop(|ui| ui.set_is_loading(false));
            return;
        };

        ui.set_current_program(ProgramInfo {
            title: episode.title.clone(),
            description: episode.description.clone(),
            start_time: "".into(),
            end_time: episode.publish_date.clone(),
            show_image: slint::Image::default(),
            program_id: 0,
            has_podcasts: false,
            episode_url: episode.share_url.clone(),
        });

        println!("Setting playback_duration to: {} seconds", episode.duration);
        ui.set_playback_duration(episode.duration as f32);
        ui.set_playback_position(0.0);
        ui.set_program_duration(0.0);

        let program_image_url = {
            let all_programs_lock = state.all_programs.lock().unwrap();
            core::podcast::get_program_image_url(&all_programs_lock, ui.get_selected_program_id())
        };

        if let Some(image_url) = program_image_url {
            let ui_clone = ui_weak.clone();
            let title = episode.title.clone();
            let desc = episode.description.clone();
            let date = episode.publish_date.clone();
            let share_url = episode.share_url.clone();

            state.spawn(async move {
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
                        episode_url: share_url,
                    });
                });
            });
        }

        let url = episode.url.to_string();
        state.set_current_episode_url(Some(url.clone()));

        let state_inner = state.clone();
        let ui_weak_for_download = ui_weak.clone();
        let ui_weak_for_playback = ui_weak.clone();
        let url_for_download = url.clone();
        let url_for_playback = url;

        state.spawn(async move {
            state_inner.stop_all_players().await;
            state_inner.set_active_player(ActivePlayer::Streaming);

            state_inner.episode_cache.start_download(
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
                            slint::Timer::single_shot(
                                std::time::Duration::from_secs(2),
                                move || {
                                    let _ = ui_for_clear.upgrade_in_event_loop(|ui| {
                                        ui.set_download_status("".into());
                                    });
                                },
                            );
                        } else {
                            ui.set_download_status(format!("Downloading ({}%)", percent).into());
                        }
                    });
                },
            );

            match state_inner
                .streaming_player
                .start_stream(url_for_playback)
                .await
            {
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

/// Favorite toggle callbacks
fn setup_favorite_callbacks(ui: &MainWindow, app_state: &AppState) {
    // Toggle favorite channel
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();

        ui.on_toggle_favorite_channel(move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let is_fav = {
                let mut fav = state.favorites.lock().unwrap();
                let r = fav.toggle_channel(id);
                let _ = fav.save();
                r
            };

            let model = ui.get_channels();
            for i in 0..model.row_count() {
                if let Some(mut item) = model.row_data(i) {
                    if item.id == id {
                        item.is_favorite = is_fav;
                        model.set_row_data(i, item.clone());
                        let favs: Vec<ChannelItem> = (0..ui.get_favorite_channels().row_count())
                            .filter_map(|j| ui.get_favorite_channels().row_data(j))
                            .filter(|c| is_fav || c.id != id)
                            .collect();
                        let mut new_favs = favs;
                        if is_fav {
                            new_favs.push(item);
                        }
                        ui.set_favorite_channels(ModelRc::from(Rc::new(VecModel::from(new_favs))));
                        break;
                    }
                }
            }
        });
    }

    // Toggle favorite program
    {
        let ui_weak = ui.as_weak();
        let state = app_state.clone();

        ui.on_toggle_favorite_program(move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let is_fav = {
                let mut fav = state.favorites.lock().unwrap();
                let r = fav.toggle_program(id);
                let _ = fav.save();
                r
            };

            let model = ui.get_programs();
            for i in 0..model.row_count() {
                if let Some(mut item) = model.row_data(i) {
                    if item.id == id {
                        item.is_favorite = is_fav;
                        model.set_row_data(i, item.clone());
                        let favs: Vec<ProgramItem> = (0..ui.get_favorite_programs().row_count())
                            .filter_map(|j| ui.get_favorite_programs().row_data(j))
                            .filter(|p| is_fav || p.id != id)
                            .collect();
                        let mut new_favs = favs;
                        if is_fav {
                            new_favs.push(item);
                        }
                        ui.set_favorite_programs(ModelRc::from(Rc::new(VecModel::from(new_favs))));
                        break;
                    }
                }
            }
        });
    }
}

/// Window dragging and share callbacks
fn setup_misc_callbacks(ui: &MainWindow, _app_state: &AppState) {
    // Window dragging
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

    // Share button - copy episode URL to clipboard
    {
        let ui_weak = ui.as_weak();

        ui.on_share_clicked(move |episode_url| {
            let url = episode_url.to_string();
            if url.is_empty() {
                println!("No episode URL available to share");
                return;
            }

            // Copy to clipboard using platform-specific approach
            #[cfg(target_os = "macos")]
            {
                use std::process::Command;
                let _ = Command::new("pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(stdin) = child.stdin.as_mut() {
                            stdin.write_all(url.as_bytes())?;
                        }
                        child.wait()
                    });
            }

            #[cfg(target_os = "windows")]
            {
                use std::process::Command;
                // Use PowerShell to set clipboard on Windows
                let _ = Command::new("powershell")
                    .args(["-command", &format!("Set-Clipboard -Value '{}'", url)])
                    .output();
            }

            #[cfg(target_os = "linux")]
            {
                use std::process::Command;
                // Try xclip first, then xsel
                let result = Command::new("xclip")
                    .args(["-selection", "clipboard"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(stdin) = child.stdin.as_mut() {
                            stdin.write_all(url.as_bytes())?;
                        }
                        child.wait()
                    });

                if result.is_err() {
                    let _ = Command::new("xsel")
                        .args(["--clipboard", "--input"])
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .and_then(|mut child| {
                            use std::io::Write;
                            if let Some(stdin) = child.stdin.as_mut() {
                                stdin.write_all(url.as_bytes())?;
                            }
                            child.wait()
                        });
                }
            }

            println!("Copied to clipboard: {}", url);

            // Show status message in UI
            if let Some(ui_handle) = ui_weak.upgrade() {
                ui_handle.set_clipboard_status(format!("Copied: {}", url).into());

                // Clear status after 2 seconds
                let ui_weak_for_clear = ui_weak.clone();
                slint::Timer::single_shot(std::time::Duration::from_secs(2), move || {
                    if let Some(ui) = ui_weak_for_clear.upgrade() {
                        ui.set_clipboard_status("".into());
                    }
                });
            }
        });
    }
}

// ============================================================================
// BACKGROUND TASKS
// ============================================================================

/// Start periodic program update task
pub fn start_program_update_task(
    ui_weak: slint::Weak<MainWindow>,
    api_client: SrApiClient,
    app_state: AppState,
) {
    let state_inner = app_state.clone();

    app_state.spawn(async move {
        let mut last_program_title: Option<String> = None;

        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

            let channel_id = state_inner.get_current_channel_id();

            if let Some(id) = channel_id {
                if let Some(new_title) = update_program_info(&ui_weak, &api_client, id).await {
                    if last_program_title.as_ref() != Some(&new_title) {
                        println!("Program changed to: {}", new_title);
                        last_program_title = Some(new_title);
                    }
                }
            } else {
                last_program_title = None;
            }
        }
    });
}
