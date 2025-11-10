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
    pub channels: Vec<Channel>, // Vec<T> is Rust's growable array type
    pub copyright: String,      // String is owned text data
    pub pagination: Pagination,
}

// Represents a single Sveriges Radio channel (like SR P1, SR P2, etc.)
//
// Note: The #[serde(rename = "...")] attribute tells serde that the JSON
// field has a different name than our Rust field. For example, the JSON
// has "liveaudio" but we name it "live_audio" in Rust (following Rust naming conventions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: u32,       // u32 = unsigned 32-bit integer (0 to 4,294,967,295)
    pub name: String,  // Channel name like "SR P1" or "SR P3"
    pub image: String, // URL to the channel's logo image

    #[serde(rename = "imagetemplate")]
    pub image_template: String, // Template URL for different image sizes

    pub color: String, // Hex color code for the channel (e.g., "#0099FF")

    #[serde(rename = "tagline")]
    pub tag_line: Option<String>, // Option<T> means this field might be null/missing

    #[serde(rename = "siteurl")]
    pub site_url: String, // URL to the channel's website

    #[serde(rename = "liveaudio")]
    pub live_audio: LiveAudio, // Contains the actual streaming URL

    #[serde(rename = "scheduleurl")]
    pub schedule_url: String, // URL to get the channel's schedule

    #[serde(rename = "channeltype")]
    pub channel_type: String, // Type of channel (e.g., "Rikskanal")

    #[serde(rename = "xmltvid")]
    pub xmltv_id: String, // XMLTV identifier for TV guide systems
}

// Contains information about the live audio stream for a channel
// This is where we get the actual MP3 stream URL to play
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveAudio {
    pub id: u32,     // Unique ID for this audio stream
    pub url: String, // THE IMPORTANT ONE: URL to the MP3 stream

    #[serde(rename = "statkey")]
    pub stat_key: String, // Key used for statistics tracking
}

// Pagination information tells us how to navigate through multiple pages
// of results (if the API returns a lot of data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub page: u32, // Current page number (starts at 1)
    pub size: u32, // Number of items per page

    #[serde(rename = "totalhits")]
    pub total_hits: u32, // Total number of results available

    #[serde(rename = "totalpages")]
    pub total_pages: u32, // Total number of pages
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
    pub pagination: Option<Pagination>, // Not present when pagination=false
}

// Contains the schedule information for a single channel
// Includes what's currently playing and what's coming next
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSchedule {
    pub id: u32,
    pub name: String,

    #[serde(rename = "currentscheduledepisode")]
    pub current_episode: Option<ScheduledEpisode>, // What's playing now (might be None)

    #[serde(rename = "nextscheduledepisode")]
    pub next_episode: Option<ScheduledEpisode>, // What's coming next (might be None)
}

// Information about a scheduled episode/program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledEpisode {
    #[serde(rename = "episodeid", default)]
    pub episode_id: Option<u32>,

    pub title: String,               // Episode title
    pub subtitle: Option<String>,    // Optional subtitle
    pub description: Option<String>, // Program description

    #[serde(rename = "starttimeutc")]
    pub start_time_utc: String, // Start time in /Date(ms)/ format

    #[serde(rename = "endtimeutc")]
    pub end_time_utc: String, // End time in /Date(ms)/ format

    pub program: Option<ProgramInfo>, // Associated program info

    #[serde(rename = "socialimage")]
    pub social_image: Option<String>, // Optional image URL
}

// Information about the program (show) that an episode belongs to
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramInfo {
    pub id: u32,
    pub name: Option<String>, // Name might be missing
}

// ============================================================================
// PROGRAMS API MODELS (for podcasts)
// ============================================================================

// Response from the programs API endpoint
// Example: https://api.sr.se/api/v2/programs?format=json
//
// This gives us all programs, including which ones have podcasts available
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramsResponse {
    pub programs: Vec<Program>,
    pub copyright: String,
    pub pagination: Option<Pagination>,
}

// Represents a Sveriges Radio program (show)
// Programs can be either live broadcasts or have podcast episodes available
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub id: u32,
    pub name: String,
    pub description: Option<String>,

    #[serde(rename = "programurl")]
    pub program_url: Option<String>,

    #[serde(rename = "programimage")]
    pub program_image: Option<String>,

    #[serde(rename = "programimagetemplate")]
    pub program_image_template: Option<String>,

    #[serde(rename = "programimagewide")]
    pub program_image_wide: Option<String>,

    #[serde(rename = "socialimage")]
    pub social_image: Option<String>,

    pub email: Option<String>,
    pub phone: Option<String>,

    pub channel: Option<ProgramChannel>,

    #[serde(rename = "haspod")]
    pub has_pod: bool, // IMPORTANT: tells us if this program has podcasts

    #[serde(rename = "hasondemand")]
    pub has_on_demand: bool, // Whether program has on-demand content

    pub archived: bool, // Whether the program is archived (no longer active)

    #[serde(rename = "programcategory", default)]
    pub program_category: Option<ProgramCategory>,
}

// Channel information within a program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramChannel {
    pub id: u32,
    pub name: String,
}

// Program category (e.g., Music, Sport, News)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramCategory {
    pub id: u32,
    pub name: String,
}


// ============================================================================
// EPISODES API MODELS
// ============================================================================

// Response from the episodes/index API endpoint
// Example: https://api.sr.se/api/v2/episodes/index?programid=3718&format=json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodesResponse {
    pub episodes: Vec<Episode>,
    pub copyright: String,
    pub pagination: Option<Pagination>,
}

// Represents a single episode from the episodes endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: u32,
    pub title: String,
    pub description: Option<String>,

    #[serde(rename = "publishdateutc")]
    pub publish_date_utc: String,

    pub url: String,

    #[serde(rename = "imageurl")]
    pub image_url: Option<String>,

    #[serde(rename = "listenpodfile")]
    pub listen_podfile: Option<EpisodePodFile>,
}

// Podfile information within an episode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodePodFile {
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub duration: u32,

    #[serde(rename = "filesizeinbytes")]
    pub file_size_in_bytes: u64,

    #[serde(rename = "publishdateutc")]
    pub publish_date_utc: String,
}
