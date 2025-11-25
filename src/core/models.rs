// Sveriges Radio API data models

use serde::{Deserialize, Serialize};

// Channels API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsResponse {
    pub channels: Vec<Channel>,
    pub copyright: String,
    pub pagination: Pagination,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: u32,
    pub name: String,
    pub image: String,
    #[serde(rename = "imagetemplate")]
    pub image_template: String,
    pub color: String,
    #[serde(rename = "tagline")]
    pub tag_line: Option<String>,
    #[serde(rename = "siteurl")]
    pub site_url: String,
    #[serde(rename = "liveaudio")]
    pub live_audio: LiveAudio,
    #[serde(rename = "scheduleurl")]
    pub schedule_url: String,
    #[serde(rename = "channeltype")]
    pub channel_type: String,
    #[serde(rename = "xmltvid")]
    pub xmltv_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveAudio {
    pub id: u32,
    pub url: String,
    #[serde(rename = "statkey")]
    pub stat_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub page: u32,
    pub size: u32,
    #[serde(rename = "totalhits")]
    pub total_hits: u32,
    #[serde(rename = "totalpages")]
    pub total_pages: u32,
}

// Schedule API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRightNowResponse {
    pub channels: Vec<ChannelSchedule>,
    pub copyright: String,
    pub pagination: Option<Pagination>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSchedule {
    pub id: u32,
    pub name: String,
    #[serde(rename = "currentscheduledepisode")]
    pub current_episode: Option<ScheduledEpisode>,
    #[serde(rename = "nextscheduledepisode")]
    pub next_episode: Option<ScheduledEpisode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledEpisode {
    #[serde(rename = "episodeid", default)]
    pub episode_id: Option<u32>,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "starttimeutc")]
    pub start_time_utc: String,
    #[serde(rename = "endtimeutc")]
    pub end_time_utc: String,
    pub program: Option<ProgramInfo>,
    #[serde(rename = "socialimage")]
    pub social_image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramInfo {
    pub id: u32,
    pub name: Option<String>,
}

// Programs API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramsResponse {
    pub programs: Vec<Program>,
    pub copyright: String,
    pub pagination: Option<Pagination>,
}

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
    pub has_pod: bool,
    #[serde(rename = "hasondemand")]
    pub has_on_demand: bool,
    pub archived: bool,
    #[serde(rename = "programcategory", default)]
    pub program_category: Option<ProgramCategory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramChannel {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramCategory {
    pub id: u32,
    pub name: String,
}

// Episodes API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodesResponse {
    pub episodes: Vec<Episode>,
    pub copyright: String,
    pub pagination: Option<Pagination>,
}

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

// Player state
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum ActivePlayer {
    #[default]
    None,
    ChannelPool,
    FilePlayer,
    Streaming,
}
