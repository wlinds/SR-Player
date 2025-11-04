// Defines all the data structures (models) that represent the JSON responses we get from Sveriges Radio's public API.
//
// We use serde to automatically convert JSON data into Rust structs.
// The #[derive(Deserialize)] attribute tells serde how to parse JSON.

use serde::{Deserialize, Serialize};

// ============================================================================
// CHANNELS API MODELS
// ============================================================================

// Main response from the channels API endpoint
// Example: https://api.sr.se/api/v2/channels?format=json
//
// The API returns a JSON object with these fields:
// - channels: an array of channel objects
// - copyright: copyright notice from SR
// - pagination: information about how many results there are
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsResponse {
    pub channels: Vec<Channel>,      // Vec<T> is Rust's growable array type
    pub copyright: String,            // String is owned text data
    pub pagination: Pagination,
}

// Represents a single Sveriges Radio channel (like SR P1, SR P2, etc.)
//
// Note: The #[serde(rename = "...")] attribute tells serde that the JSON
// field has a different name than our Rust field. For example, the JSON
// has "liveaudio" but we name it "live_audio" in Rust (following Rust naming conventions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: u32,                      // u32 = unsigned 32-bit integer (0 to 4,294,967,295)
    pub name: String,                 // Channel name like "SR P1" or "SR P3"
    pub image: String,                // URL to the channel's logo image

    #[serde(rename = "imagetemplate")]
    pub image_template: String,       // Template URL for different image sizes

    pub color: String,                // Hex color code for the channel (e.g., "#0099FF")

    #[serde(rename = "tagline")]
    pub tag_line: Option<String>,     // Option<T> means this field might be null/missing

    #[serde(rename = "siteurl")]
    pub site_url: String,             // URL to the channel's website

    #[serde(rename = "liveaudio")]
    pub live_audio: LiveAudio,        // Contains the actual streaming URL

    #[serde(rename = "scheduleurl")]
    pub schedule_url: String,         // URL to get the channel's schedule

    #[serde(rename = "channeltype")]
    pub channel_type: String,         // Type of channel (e.g., "Rikskanal")

    #[serde(rename = "xmltvid")]
    pub xmltv_id: String,             // XMLTV identifier for TV guide systems
}

// Contains information about the live audio stream for a channel
// This is where we get the actual MP3 stream URL to play
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveAudio {
    pub id: u32,                      // Unique ID for this audio stream
    pub url: String,                  // THE IMPORTANT ONE: URL to the MP3 stream

    #[serde(rename = "statkey")]
    pub stat_key: String,             // Key used for statistics tracking
}

// Pagination information tells us how to navigate through multiple pages
// of results (if the API returns a lot of data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub page: u32,                    // Current page number (starts at 1)
    pub size: u32,                    // Number of items per page

    #[serde(rename = "totalhits")]
    pub total_hits: u32,              // Total number of results available

    #[serde(rename = "totalpages")]
    pub total_pages: u32,             // Total number of pages
}

// ============================================================================
// SCHEDULE API MODELS
// ============================================================================

// Response from the schedule rightnow endpoint
// Example: https://api.sr.se/api/v2/scheduledepisodes/rightnow?format=json
//
// This gives us information about what's currently playing on each channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRightNowResponse {
    pub channels: Vec<ChannelSchedule>,
    pub copyright: String,
    pub pagination: Option<Pagination>,  // Not present when pagination=false
}

// Contains the schedule information for a single channel
// Includes what's currently playing and what's coming next
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSchedule {
    pub id: u32,
    pub name: String,

    #[serde(rename = "currentscheduledepisode")]
    pub current_episode: Option<ScheduledEpisode>,  // What's playing now (might be None)

    #[serde(rename = "nextscheduledepisode")]
    pub next_episode: Option<ScheduledEpisode>,     // What's coming next (might be None)
}

// Information about a scheduled episode/program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledEpisode {
    #[serde(rename = "episodeid", default)]
    pub episode_id: Option<u32>,

    pub title: String,                              // Episode title
    pub subtitle: Option<String>,                   // Optional subtitle
    pub description: Option<String>,                // Program description

    #[serde(rename = "starttimeutc")]
    pub start_time_utc: String,                     // Start time in /Date(ms)/ format

    #[serde(rename = "endtimeutc")]
    pub end_time_utc: String,                       // End time in /Date(ms)/ format

    pub program: Option<ProgramInfo>,               // Associated program info

    #[serde(rename = "socialimage")]
    pub social_image: Option<String>,               // Optional image URL
}

// Information about the program (show) that an episode belongs to
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramInfo {
    pub id: u32,
    pub name: Option<String>,                       // Name might be missing
}

