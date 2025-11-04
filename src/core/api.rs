// This module handles all communication with the Sveriges Radio API
// https://www.sverigesradio.se/oppetapi
// We use the 'reqwest' library to make HTTP requests and 'anyhow' for error handling.

use anyhow::{Context, Result};
use log::{debug, info};

use crate::core::models::{Channel, ChannelsResponse, ScheduleRightNowResponse};

// Base URL for all Sveriges Radio API endpoints
const SR_API_BASE: &str = "https://api.sr.se/api/v2";

// ============================================================================
// API CLIENT STRUCT
// ============================================================================

/// The main API client that handles all requests to Sveriges Radio
///
/// In Rust, we use a struct to group related data and functionality.
/// This client uses reqwest's blocking client, which means it will wait for the HTTP request to complete before continuing (simpler for beginners).
#[derive(Clone)]
pub struct SrApiClient {
    client: reqwest::blocking::Client, // The HTTP client that makes requests
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
        Ok(Self { client })
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
        let url = format!(
            "{}/scheduledepisodes/rightnow?format=json&pagination=false",
            SR_API_BASE
        );

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

        Ok(schedule_response)
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
