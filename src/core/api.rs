// Sveriges Radio API client with caching

use anyhow::{Context, Result};
use log::{debug, info};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::core::models::{
    Channel, ChannelsResponse, Episode, EpisodesResponse, Program, ProgramsResponse,
    ScheduleRightNowResponse, SingleChannelScheduleResponse,
};

const SR_API_BASE: &str = "https://api.sr.se/api/v2";
const SCHEDULE_CACHE_TTL: Duration = Duration::from_secs(30);
const PROGRAMS_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const EPISODES_CACHE_TTL: Duration = Duration::from_secs(60 * 60);

struct CacheEntry<T> {
    data: T,
    fetched_at: Instant,
}

impl<T> CacheEntry<T> {
    fn new(data: T) -> Self {
        Self {
            data,
            fetched_at: Instant::now(),
        }
    }
    fn is_valid(&self, ttl: Duration) -> bool {
        self.fetched_at.elapsed() < ttl
    }
    fn age_secs(&self) -> f32 {
        self.fetched_at.elapsed().as_secs_f32()
    }
}

struct ScheduleCache {
    data: Option<ScheduleRightNowResponse>,
    last_fetch: Option<Instant>,
}

#[derive(Clone)]
pub struct SrApiClient {
    client: reqwest::blocking::Client,
    schedule_cache: Arc<Mutex<ScheduleCache>>,
    programs_cache: Arc<Mutex<Option<CacheEntry<Vec<Program>>>>>,
    episodes_cache: Arc<Mutex<HashMap<u32, CacheEntry<Vec<Episode>>>>>,
}

impl SrApiClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent("SR-Player/0.1.0")
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        info!("SR API client initialized");

        Ok(Self {
            client,
            schedule_cache: Arc::new(Mutex::new(ScheduleCache {
                data: None,
                last_fetch: None,
            })),
            programs_cache: Arc::new(Mutex::new(None)),
            episodes_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn get_channels(&self) -> Result<Vec<Channel>> {
        let url = format!(
            "{}/channels?liveaudiotemplateid=5&audioquality=hi&format=json",
            SR_API_BASE
        );
        debug!("Fetching channels from: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch channels")?;
        let channels_response: ChannelsResponse =
            response.json().context("Failed to parse channels JSON")?;

        info!("Fetched {} channels", channels_response.channels.len());
        Ok(channels_response.channels)
    }

    #[allow(dead_code)]
    pub fn get_schedule_right_now(&self) -> Result<ScheduleRightNowResponse> {
        // Check cache
        {
            let cache = self.schedule_cache.lock().unwrap();
            if let (Some(data), Some(last_fetch)) = (&cache.data, cache.last_fetch) {
                if last_fetch.elapsed() < SCHEDULE_CACHE_TTL {
                    debug!(
                        "Using cached schedule (age: {:.1}s)",
                        last_fetch.elapsed().as_secs_f32()
                    );
                    return Ok(data.clone());
                }
            }
        }

        let url = format!(
            "{}/scheduledepisodes/rightnow?format=json&pagination=false",
            SR_API_BASE
        );
        info!("Fetching fresh schedule");

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch schedule")?;
        let schedule: ScheduleRightNowResponse =
            response.json().context("Failed to parse schedule JSON")?;

        info!("Fetched schedule for {} channels", schedule.channels.len());

        // Update cache
        {
            let mut cache = self.schedule_cache.lock().unwrap();
            cache.data = Some(schedule.clone());
            cache.last_fetch = Some(Instant::now());
        }

        Ok(schedule)
    }

    /// Fetch the current schedule for a single channel (smaller payload than all channels)
    pub fn get_schedule_for_channel(&self, channel_id: i32) -> Result<ScheduleRightNowResponse> {
        // Check cache first - if all-channels cache is fresh, use that
        {
            let cache = self.schedule_cache.lock().unwrap();
            if let (Some(data), Some(last_fetch)) = (&cache.data, cache.last_fetch) {
                if last_fetch.elapsed() < SCHEDULE_CACHE_TTL {
                    debug!(
                        "Using cached schedule for channel {} (age: {:.1}s)",
                        channel_id,
                        last_fetch.elapsed().as_secs_f32()
                    );
                    return Ok(data.clone());
                }
            }
        }

        let url = format!(
            "{}/scheduledepisodes/rightnow?channelid={}&format=json&pagination=false",
            SR_API_BASE, channel_id
        );
        info!("Fetching schedule for channel {}", channel_id);

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch channel schedule")?;

        // Single-channel endpoint returns {"channel": {...}} instead of {"channels": [...]}
        let single: SingleChannelScheduleResponse =
            response.json().context("Failed to parse channel schedule JSON")?;

        Ok(ScheduleRightNowResponse {
            copyright: single.copyright,
            channels: vec![single.channel],
            pagination: None,
        })
    }

    pub fn get_programs_with_podcasts(&self) -> Result<Vec<Program>> {
        // Check cache
        {
            let cache = self.programs_cache.lock().unwrap();
            if let Some(entry) = cache.as_ref() {
                if entry.is_valid(PROGRAMS_CACHE_TTL) {
                    debug!("Using cached programs (age: {:.1}s)", entry.age_secs());
                    return Ok(entry.data.clone());
                }
            }
        }

        let url = format!("{}/programs?format=json&pagination=false", SR_API_BASE);
        info!("Fetching fresh programs");

        let response = self
            .client
            .get(&url)
            .send()
            .context("Failed to fetch programs")?;
        let programs_response: ProgramsResponse =
            response.json().context("Failed to parse programs JSON")?;

        let podcast_programs: Vec<Program> = programs_response
            .programs
            .into_iter()
            .filter(|p| p.has_pod && !p.archived)
            .collect();

        info!("Fetched {} programs with podcasts", podcast_programs.len());

        // Update cache
        {
            let mut cache = self.programs_cache.lock().unwrap();
            *cache = Some(CacheEntry::new(podcast_programs.clone()));
        }

        Ok(podcast_programs)
    }

    pub fn get_podcast_episodes(&self, program_id: u32) -> Result<Vec<Episode>> {
        // Check cache
        {
            let cache = self.episodes_cache.lock().unwrap();
            if let Some(entry) = cache.get(&program_id) {
                if entry.is_valid(EPISODES_CACHE_TTL) {
                    debug!(
                        "Using cached episodes for program {} (age: {:.1}s)",
                        program_id,
                        entry.age_secs()
                    );
                    return Ok(entry.data.clone());
                }
            }
        }

        let url = format!(
            "{}/episodes/index?programid={}&format=json&pagination=true&page=1&size=25&sort=publishdateutc%2Bdesc",
            SR_API_BASE, program_id
        );
        info!("Fetching episodes for program {}", program_id);

        let response = self.client.get(&url).send().context(format!(
            "Failed to fetch episodes for program {}",
            program_id
        ))?;
        let episodes_response: EpisodesResponse =
            response.json().context("Failed to parse episodes JSON")?;

        let episodes: Vec<Episode> = episodes_response
            .episodes
            .into_iter()
            .filter(|e| e.listen_podfile.is_some())
            .collect();

        info!(
            "Fetched {} episodes for program {}",
            episodes.len(),
            program_id
        );

        // Update cache
        {
            let mut cache = self.episodes_cache.lock().unwrap();
            cache.insert(program_id, CacheEntry::new(episodes.clone()));
        }

        Ok(episodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_client_creation() {
        assert!(SrApiClient::new().is_ok());
    }

    #[test]
    #[ignore]
    fn test_fetch_channels() {
        let client = SrApiClient::new().unwrap();
        let channels = client.get_channels().unwrap();
        assert!(!channels.is_empty());
    }

    #[test]
    #[ignore]
    fn test_fetch_schedule() {
        let client = SrApiClient::new().unwrap();
        let schedule = client.get_schedule_right_now().unwrap();
        assert!(!schedule.channels.is_empty());
    }
}
