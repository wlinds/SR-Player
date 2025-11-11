// Downloads episodes in the background while streaming plays
// Allows seeking once download completes

use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::Result;
use log::info;

#[derive(Clone)]
pub struct EpisodeCache {
    // Map of episode URL to downloaded bytes
    cache: Arc<Mutex<HashMap<String, Bytes>>>,
}

impl EpisodeCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // Start downloading an episode in the background with progress callback
    pub fn start_download<F>(&self, url: String, progress_callback: F)
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let cache = self.cache.clone();
        let progress = Arc::new(progress_callback);

        tokio::spawn(async move {
            info!("Starting background download: {}", url);

            match download_episode_with_progress(&url, progress).await {
                Ok(data) => {
                    info!("Download complete: {} ({} bytes)", url, data.len());
                    let mut cache_lock = cache.lock().await;
                    cache_lock.insert(url.clone(), data);
                }
                Err(e) => {
                    eprintln!("Failed to download episode {}: {}", url, e);
                }
            }
        });
    }

    pub async fn is_downloaded(&self, url: &str) -> bool {
        let cache = self.cache.lock().await;
        cache.contains_key(url)
    }

    // Get downloaded episode data
    pub async fn get(&self, url: &str) -> Option<Bytes> {
        let cache = self.cache.lock().await;
        cache.get(url).cloned()
    }
}

async fn download_episode_with_progress<F>(url: &str, progress_callback: Arc<F>) -> Result<Bytes>
where
    F: Fn(u64, u64) + Send + Sync,
{
    let client = reqwest::Client::builder()
        .user_agent("SR-Player/3.0-EpisodeCache")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;

    let response = client.get(url).send().await?;

    // Get total size from Content-Length header
    let total_size = response.content_length().unwrap_or(0);

    // Download with progress tracking
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut data = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        data.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;

        // Report progress
        progress_callback(downloaded, total_size);
    }

    Ok(Bytes::from(data))
}
