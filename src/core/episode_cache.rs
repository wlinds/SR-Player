// Downloads episodes in the background while streaming plays
// Allows seeking once download completes

use anyhow::Result;
use bytes::Bytes;
use log::info;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

// Maximum total cache size in bytes (500 MB)
const MAX_CACHE_SIZE_BYTES: usize = 500 * 1024 * 1024;

struct CacheEntry {
    data: Bytes,
}

#[derive(Clone)]
pub struct EpisodeCache {
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    // Insertion order for LRU eviction (oldest first)
    order: Arc<Mutex<VecDeque<String>>>,
    total_size: Arc<Mutex<usize>>,
}

impl EpisodeCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            order: Arc::new(Mutex::new(VecDeque::new())),
            total_size: Arc::new(Mutex::new(0)),
        }
    }

    /// Evict oldest entries until total size is under the limit
    async fn evict_if_needed(
        cache: &Mutex<HashMap<String, CacheEntry>>,
        order: &Mutex<VecDeque<String>>,
        total_size: &Mutex<usize>,
    ) {
        let mut cache_lock = cache.lock().await;
        let mut order_lock = order.lock().await;
        let mut size_lock = total_size.lock().await;

        while *size_lock > MAX_CACHE_SIZE_BYTES {
            let Some(oldest_url) = order_lock.pop_front() else {
                break;
            };
            if let Some(entry) = cache_lock.remove(&oldest_url) {
                *size_lock = size_lock.saturating_sub(entry.data.len());
                info!(
                    "Evicted cached episode: {} ({:.1} MB), total cache: {:.1} MB",
                    oldest_url,
                    entry.data.len() as f64 / 1_048_576.0,
                    *size_lock as f64 / 1_048_576.0
                );
            }
        }
    }

    // Start downloading an episode in the background with progress callback
    pub fn start_download<F>(&self, url: String, progress_callback: F)
    where
        F: Fn(u64, u64) + Send + Sync + 'static,
    {
        let cache = self.cache.clone();
        let order = self.order.clone();
        let total_size = self.total_size.clone();
        let progress = Arc::new(progress_callback);

        tokio::spawn(async move {
            info!("Starting background download: {}", url);

            match download_episode_with_progress(&url, progress).await {
                Ok(data) => {
                    let data_len = data.len();
                    info!("Download complete: {} ({} bytes)", url, data_len);

                    // Insert into cache
                    {
                        let mut cache_lock = cache.lock().await;
                        let mut order_lock = order.lock().await;
                        let mut size_lock = total_size.lock().await;

                        // If URL already cached, remove old size
                        if let Some(old) = cache_lock.remove(&url) {
                            *size_lock = size_lock.saturating_sub(old.data.len());
                            order_lock.retain(|u| u != &url);
                        }

                        cache_lock.insert(url.clone(), CacheEntry { data });
                        order_lock.push_back(url.clone());
                        *size_lock += data_len;
                    }

                    // Evict if over limit
                    Self::evict_if_needed(&cache, &order, &total_size).await;
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
        cache.get(url).map(|entry| entry.data.clone())
    }
}

/// Shared async HTTP client for episode downloads (connection pooling)
fn shared_download_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("SR-Player/3.0-EpisodeCache")
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create download HTTP client")
    })
}

async fn download_episode_with_progress<F>(url: &str, progress_callback: Arc<F>) -> Result<Bytes>
where
    F: Fn(u64, u64) + Send + Sync,
{
    let response = shared_download_client().get(url).send().await?;

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
