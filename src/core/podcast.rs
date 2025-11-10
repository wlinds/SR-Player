// Podcast Module - Helper functions for podcast/program management
//
// This module provides utility functions for:
// - Fetching programs and episodes from the SR API
// - Converting API models (Program, PodFile) to UI models (ProgramItem, EpisodeItem)
// - Organizing programs into groups (Podcasts vs News tabs)
// - Handling expandable groups (P4 Local News, Ekot News)

use anyhow::Result;
use slint::Image;

use crate::core::api::SrApiClient;
use crate::core::models::{PodFile, Program};
use crate::{EpisodeItem, ProgramItem};

/// Parse SR API date format /Date(milliseconds)/ and convert to a readable format
/// Example: "/Date(1762431000000)/" -> "11 Aug 2020"
fn parse_sr_date_to_display(date_str: &str) -> String {
    use chrono::DateTime;

    if let Some(ms_str) = date_str
        .strip_prefix("/Date(")
        .and_then(|s| s.strip_suffix(")/"))
    {
        if let Ok(ms) = ms_str.parse::<i64>() {
            if let Some(dt) = DateTime::from_timestamp_millis(ms) {
                return dt.format("%d %b %Y").to_string();
            }
        }
    }
    String::from("Unknown")
}

/// Convert a PodFile to an EpisodeItem (for Slint UI)
pub fn episode_to_item(episode: &PodFile) -> Option<EpisodeItem> {
    i32::try_from(episode.id).ok().map(|id| {
        // Format file size (bytes to MB)
        let file_size_mb = episode.file_size_in_bytes as f64 / 1_048_576.0;
        let file_size_str = format!("{:.1} MB", file_size_mb);

        // Format publish date
        let publish_date = parse_sr_date_to_display(&episode.publish_date_utc);

        EpisodeItem {
            id,
            title: episode.title.clone().into(),
            description: episode.description.clone().unwrap_or_default().into(),
            duration: episode.duration as i32,
            publish_date: publish_date.into(),
            url: episode.url.clone().into(),
            file_size: file_size_str.into(),
        }
    })
}

/// Fetch all programs with podcasts and return the raw Program data
pub async fn fetch_programs_with_podcasts(
    api_client: &SrApiClient,
) -> Result<Vec<Program>> {
    let api_clone = api_client.clone();

    let programs = tokio::task::spawn_blocking(move || api_clone.get_programs_with_podcasts())
        .await??;

    println!("Fetched {} programs with podcasts", programs.len());

    Ok(programs)
}

/// Convert programs to ProgramItems for Podcasts tab
///
/// Filters out news programs (P4 Local, Ekot) which belong in the News tab.
/// All podcast programs are shown as regular items (no grouping).
pub fn programs_to_items_podcasts(programs: &[Program]) -> Vec<ProgramItem> {
    let mut result = Vec::new();

    // Filter out P4 and Ekot programs (they belong in News tab)
    let non_news_programs: Vec<_> = programs
        .iter()
        .filter(|p| {
            !p.name.starts_with("Nyheter P4 ") &&
            !p.name.starts_with("Ekot") &&
            !p.name.contains("Ekonomiekot")
        })
        .collect();

    // Add all non-news programs
    for program in non_news_programs {
        if let Ok(id) = i32::try_from(program.id) {
            let description = program
                .description
                .clone()
                .unwrap_or_default()
                .chars()
                .take(50)
                .collect::<String>();

            result.push(ProgramItem {
                id,
                name: program.name.clone().into(),
                description: description.into(),
                image: Image::default(),
                has_pod: program.has_pod,
                is_group: false,
                is_expanded: true, // Regular programs are always visible
                is_child: false,
                group_id: 0,
                group_expanded: false, // Not a group
            });
        }
    }

    result
}

/// Convert programs to ProgramItems for News tab with expandable grouping
///
/// Creates two collapsible groups:
/// 1. "Ekot News" (group_id: -2) - Contains Ekot and Ekonomiekot programs
/// 2. "P4 Local News" (group_id: -1) - Contains all regional P4 news programs
///
/// The groups_expanded HashMap tracks which groups are currently expanded.
pub fn programs_to_items_news(programs: &[Program], groups_expanded: &std::collections::HashMap<i32, bool>) -> Vec<ProgramItem> {
    let mut result = Vec::new();

    // Separate P4, Ekot, and other programs
    let (p4_programs, remaining): (Vec<_>, Vec<_>) = programs
        .iter()
        .partition(|p| p.name.starts_with("Nyheter P4 "));

    let (ekot_programs, _other_programs): (Vec<&Program>, Vec<&Program>) = remaining
        .into_iter()
        .partition(|p| p.name.starts_with("Ekot") || p.name.contains("Ekonomiekot"));

    // Add Ekot group first (news at top)
    if !ekot_programs.is_empty() {
        let ekot_group_id = -2; // Use negative ID for groups (different from P4)
        let ekot_expanded = groups_expanded.get(&ekot_group_id).copied().unwrap_or(false);

        // Add Ekot group header
        result.push(ProgramItem {
            id: ekot_group_id,
            name: "Ekot News".into(),
            description: "".into(),
            image: Image::default(),
            has_pod: true,
            is_group: true,
            is_expanded: true, // Group headers are always visible
            is_child: false,
            group_id: 0,
            group_expanded: ekot_expanded, // Shows the expansion state
        });

        // Add Ekot children if expanded
        if ekot_expanded {
            for program in ekot_programs {
                if let Ok(id) = i32::try_from(program.id) {
                    result.push(ProgramItem {
                        id,
                        name: program.name.clone().into(),
                        description: "News podcasts".into(),
                        image: Image::default(),
                        has_pod: program.has_pod,
                        is_group: false,
                        is_expanded: ekot_expanded, // Child visibility depends on parent group state
                        is_child: true,
                        group_id: ekot_group_id,
                        group_expanded: false, // Children are not groups
                    });
                }
            }
        }
    }

    // Add P4 group if there are P4 programs
    if !p4_programs.is_empty() {
        let p4_group_id = -1; // Use negative ID for groups
        let p4_expanded = groups_expanded.get(&p4_group_id).copied().unwrap_or(false);

        // Add P4 group header
        result.push(ProgramItem {
            id: p4_group_id,
            name: "P4 Local News".into(),
            description: "".into(),
            image: Image::default(),
            has_pod: true,
            is_group: true,
            is_expanded: true, // Group headers are always visible
            is_child: false,
            group_id: 0,
            group_expanded: p4_expanded, // Shows the expansion state
        });

        // Add P4 children if expanded
        if p4_expanded {
            for program in p4_programs {
                if let Ok(id) = i32::try_from(program.id) {
                    let region_name = program.name
                        .strip_prefix("Nyheter P4 ")
                        .unwrap_or(&program.name);

                    result.push(ProgramItem {
                        id,
                        name: region_name.to_string().into(),
                        description: "Local news podcasts".into(),
                        image: Image::default(),
                        has_pod: program.has_pod,
                        is_group: false,
                        is_expanded: p4_expanded, // Child visibility depends on parent group state
                        is_child: true,
                        group_id: p4_group_id,
                        group_expanded: false, // Children are not groups
                    });
                }
            }
        }
    }

    // News tab only shows Ekot and P4 programs, no other podcasts
    result
}

/// Fetch episodes for a specific program and return as EpisodeItem vector
pub async fn fetch_episodes_for_program(
    api_client: &SrApiClient,
    program_id: u32,
) -> Result<(Vec<EpisodeItem>, String)> {
    let api_clone = api_client.clone();

    let episodes = tokio::task::spawn_blocking(move || api_clone.get_podcast_episodes(program_id))
        .await??;

    println!("Fetched {} episodes for program {}", episodes.len(), program_id);

    // Convert to Slint EpisodeItem structs
    let episode_items: Vec<EpisodeItem> = episodes
        .iter()
        .filter_map(episode_to_item)
        .collect();

    // Get program name for the header
    let program_name = if !episodes.is_empty() {
        episodes[0]
            .program
            .name
            .clone()
            .unwrap_or_else(|| String::from("Podcast"))
    } else {
        String::from("Podcast")
    };

    Ok((episode_items, program_name))
}
