// This module handles all communication with the Sveriges Radio API
// https://www.sverigesradio.se/oppetapi
//
// This module handles all HTTP communication with the SR API endpoints.
// We use:
// - reqwest: For making HTTP requests (blocking client for simplicity)
// - anyhow: For ergonomic error handling with context
// - serde: For JSON deserialization into our model structs

use anyhow::{Context, Result};
use log::{debug, info};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::core::models::{
    Channel, ChannelsResponse, Episode, EpisodesResponse, Program, ProgramsResponse,
    ScheduleRightNowResponse,
};

// Base URL for all Sveriges Radio API endpoints
const SR_API_BASE: &str = "https://api.sr.se/api/v2";

// ============================================================================
// API CLIENT STRUCT
// ============================================================================

/// Cached schedule data with timestamp
struct ScheduleCache {
    data: Option<ScheduleRightNowResponse>,
    last_fetch: Option<Instant>,
}

/// The main API client that handles all requests to Sveriges Radio
///
/// Uses a blocking HTTP client for simplicity. All methods are synchronous and should
/// be called from within tokio::task::spawn_blocking when used in async contexts.
#[derive(Clone)]
pub struct SrApiClient {
    client: reqwest::blocking::Client, // The HTTP client that makes requests
    schedule_cache: Arc<Mutex<ScheduleCache>>, // Cache schedule data (refreshed every 30 seconds)
}

// 'impl' blocks define methods (functions) for a struct
impl SrApiClient {
    /// Creates a new API client
    ///
    /// Example usage:
    /// ```
    /// let client = SrApiClient::new();
    /// ```
    pub fn new() -> Result<Self> {
        // Build the HTTP client with a user agent (identifies our app to the API)
        let client = reqwest::blocking::Client::builder()
            .user_agent("SR-Player/0.1.0") // Identify ourselves to the API
            .timeout(std::time::Duration::from_secs(30)) // Wait max 30 seconds for response
            .build()
            .context("Failed to create HTTP client")?; // The ? operator propagates errors

        info!("SR API client initialized");

        // 'Self' refers to the struct we're implementing (SrApiClient)
        Ok(Self {
            client,
            schedule_cache: Arc::new(Mutex::new(ScheduleCache {
                data: None,
                last_fetch: None,
            })),
        })
    }

    /// Fetches all available Sveriges Radio channels
    ///
    /// Returns a Vec (array) of Channel structs
    ///
    /// Example:
    /// ```
    /// let channels = client.get_channels()?;
    /// for channel in channels {
    ///     println!("Channel: {} - Stream URL: {}", channel.name, channel.live_audio.url);
    /// }
    /// ```
    pub fn get_channels(&self) -> Result<Vec<Channel>> {
        // Build the full URL with AAC stream template and high quality
        // liveaudiotemplateid=5 gives us raw HTTP AAC streams (not playlists!)
        // audioquality=hi gives us 320kbps AAC (highest quality)
        let url = format!(
            "{}/channels?liveaudiotemplateid=5&audioquality=hi&format=json",
            SR_API_BASE
        );

        debug!("Fetching channels from: {}", url);

        // Make the HTTP GET request
        // The '?' at the end means "if this fails, return the error immediately"
        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch channels")?;

        // Parse the JSON response into our ChannelsResponse struct
        // serde_json automatically converts JSON to Rust structs
        let channels_response: ChannelsResponse = response
            .json()
            .context("Failed to parse channels JSON response")?;

        info!(
            "Successfully fetched {} channels",
            channels_response.channels.len()
        );

        // Return just the channels array, not the whole response
        Ok(channels_response.channels)
    }

    /// Fetches the current schedule for all channels (what's playing right now)
    ///
    /// Returns a ScheduleRightNowResponse containing current and next episodes
    ///
    /// Example:
    /// ```
    /// let schedule = client.get_schedule_right_now()?;
    /// for channel in schedule.channels {
    ///     if let Some(episode) = channel.current_episode {
    ///         println!("{}: {}", channel.name, episode.title);
    ///     }
    /// }
    /// ```
    pub fn get_schedule_right_now(&self) -> Result<ScheduleRightNowResponse> {
        // Check cache first (30 second TTL)
        const CACHE_TTL: Duration = Duration::from_secs(30);

        {
            let cache = self.schedule_cache.lock().unwrap();
            if let (Some(data), Some(last_fetch)) = (&cache.data, cache.last_fetch) {
                if last_fetch.elapsed() < CACHE_TTL {
                    debug!(
                        "Using cached schedule (age: {:.1}s)",
                        last_fetch.elapsed().as_secs_f32()
                    );
                    return Ok(data.clone());
                }
            }
        }

        // Cache miss or expired - fetch fresh data
        let url = format!(
            "{}/scheduledepisodes/rightnow?format=json&pagination=false",
            SR_API_BASE
        );

        info!("Fetching fresh schedule from API (cache expired or empty)");
        debug!("Fetching current schedule from: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch schedule")?;

        let schedule_response: ScheduleRightNowResponse = response
            .json()
            .context("Failed to parse schedule JSON response")?;

        info!(
            "Successfully fetched schedule for {} channels",
            schedule_response.channels.len()
        );

        // Update cache
        {
            let mut cache = self.schedule_cache.lock().unwrap();
            cache.data = Some(schedule_response.clone());
            cache.last_fetch = Some(Instant::now());
        }

        Ok(schedule_response)
    }

    /// Fetches all programs with podcasts available
    ///
    /// Returns a Vec of Program structs filtered to only include those with podcasts
    ///
    /// Example:
    /// ```
    /// let programs = client.get_programs_with_podcasts()?;
    /// for program in programs {
    ///     println!("Program: {} (ID: {})", program.name, program.id);
    /// }
    /// ```
    pub fn get_programs_with_podcasts(&self) -> Result<Vec<Program>> {
        // Fetch all programs without pagination (all results at once)
        // We'll filter for has_pod=true after fetching
        let url = format!("{}/programs?format=json&pagination=false", SR_API_BASE);

        debug!("Fetching programs from: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch programs")?;

        let programs_response: ProgramsResponse = response
            .json()
            .context("Failed to parse programs JSON response")?;

        // Filter to only include programs that have podcasts and are not archived
        let podcast_programs: Vec<Program> = programs_response
            .programs
            .into_iter()
            .filter(|p| p.has_pod && !p.archived)
            .collect();

        info!(
            "Successfully fetched {} programs with podcasts",
            podcast_programs.len()
        );

        Ok(podcast_programs)
    }

    /// Fetches the 25 most recent podcast episodes for a specific program
    ///
    /// Uses the episodes/index endpoint which supports reliable server-side sorting.
    /// Episodes are sorted by publish date (newest first) and limited to 25 results.
    ///
    /// Returns a Vec of Episode structs
    ///
    /// Example:
    /// ```
    /// let episodes = client.get_podcast_episodes(4916)?;
    /// for episode in episodes {
    ///     println!("Episode: {}", episode.title);
    /// }
    /// ```
    pub fn get_podcast_episodes(&self, program_id: u32) -> Result<Vec<Episode>> {
        // Use episodes/index endpoint for reliable server-side sorting
        // sort=publishdateutc%2Bdesc (%2B = URL-encoded '+') gives newest episodes first
        let url = format!(
            "{}/episodes/index?programid={}&format=json&pagination=true&page=1&size=25&sort=publishdateutc%2Bdesc",
            SR_API_BASE, program_id
        );

        debug!("Fetching podcast episodes from: {}", url);

        let response = self.client.get(&url).send().context(format!(
            "Failed to fetch podcast episodes for program {}",
            program_id
        ))?;

        let episodes_response: EpisodesResponse = response
            .json()
            .context("Failed to parse podcast episodes JSON response")?;

        info!(
            "Fetched {} podcast episodes for program {} (server-side sorted)",
            episodes_response.episodes.len(),
            program_id
        );

        // Filter out episodes without podcast files
        let episodes: Vec<Episode> = episodes_response
            .episodes
            .into_iter()
            .filter(|episode| episode.listen_podfile.is_some())
            .collect();

        Ok(episodes)
    }
}

// ============================================================================
// TESTS
// ============================================================================

// They only run when we execute 'cargo test'
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_client_creation() {
        // This test just checks that we can create an API client without errors
        let client = SrApiClient::new();
        assert!(client.is_ok(), "Failed to create API client");
    }

    // Note: The tests below make real network requests to SR's API
    // In a production app, we should (?) mock these to avoid network calls during testing

    #[test]
    #[ignore] // Ignored by default because it makes a real network request
    fn test_fetch_channels() {
        let client = SrApiClient::new().unwrap();
        let channels = client.get_channels().unwrap();

        assert!(!channels.is_empty(), "Should fetch at least one channel");

        // Verify that each channel has required fields
        for channel in &channels {
            assert!(!channel.name.is_empty(), "Channel should have a name");
            assert!(
                !channel.live_audio.url.is_empty(),
                "Channel should have a live audio URL"
            );
        }
    }

    #[test]
    #[ignore] // Ignored by default because it makes a real network request
    fn test_fetch_schedule() {
        let client = SrApiClient::new().unwrap();
        let schedule = client.get_schedule_right_now().unwrap();

        assert!(
            !schedule.channels.is_empty(),
            "Should fetch schedule for at least one channel"
        );

        // Check that at least one channel has a current episode
        let has_current = schedule
            .channels
            .iter()
            .any(|ch| ch.current_episode.is_some());
        assert!(
            has_current,
            "At least one channel should have a current episode"
        );
    }
}
