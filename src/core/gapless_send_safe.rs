// Send-safe wrapper for GaplessPlayer
//
// The core problem: rodio's OutputStream contains raw pointers (*mut ()) which aren't Send.
// Solution: Keep OutputStream on a dedicated thread, communicate via channels.
//
// Architecture:
// UI Thread -> Commands (mpsc) -> Audio Thread (owns OutputStream/Sink) -> Playback

use crate::core::gapless_streaming::StreamStats;
use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

// Commands that can be sent to the audio thread
#[derive(Debug)]
#[allow(dead_code)] // Some variants reserved for future UI controls
pub enum AudioCommand {
    StartStream {
        url: String,
        response: oneshot::Sender<Result<()>>,
    },
    Stop {
        response: oneshot::Sender<()>,
    },
    Pause,
    Resume,
    SetVolume(f32),
    GetStats {
        response: oneshot::Sender<StreamStats>,
    },
    SeekTo {
        offset_secs: f32,
        response: oneshot::Sender<Result<f32>>,
    },
    GetBufferDepth {
        response: oneshot::Sender<f32>,
    },
    GetBufferTimeRange {
        response: oneshot::Sender<(f32, f32)>,
    },
    CanSeekTo {
        offset_secs: f32,
        response: oneshot::Sender<bool>,
    },
    GetBufferDataFromOffset {
        offset_secs: f32,
        response: oneshot::Sender<Result<Vec<u8>>>,
    },
    IsAtLiveEdge {
        response: oneshot::Sender<bool>,
    },
    SetAtLiveEdge {
        at_live: bool,
    },
    Shutdown,
}

// Send-safe wrapper for GaplessPlayer
// This can be safely shared across threads and used in async contexts
pub struct SendSafeGaplessPlayer {
    command_tx: mpsc::UnboundedSender<AudioCommand>,
}

impl SendSafeGaplessPlayer {
    // Create a new Send-safe gapless player with an optional shared HTTP client
    // Spawns a dedicated audio thread that owns the non-Send OutputStream
    pub fn new_with_client(http_client: Option<reqwest::Client>) -> Result<Self> {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<AudioCommand>();

        // Spawn dedicated audio thread
        std::thread::spawn(move || {
            // Create tokio runtime for this thread
            let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

            rt.block_on(async {
                // Import inside the thread to keep it thread-local
                use crate::core::gapless_streaming::GaplessPlayer;

                let mut player = match GaplessPlayer::new_with_client(http_client) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to create GaplessPlayer: {}", e);
                        return;
                    }
                };

                // Note: Pre-warming disabled - connection pooling doesn't help with SR's slow servers
                // The channel pool keeps streams alive instead for instant switching
                // player.prewarm_connections().await;

                // Process commands from UI thread
                while let Some(cmd) = command_rx.recv().await {
                    match cmd {
                        AudioCommand::StartStream { url, response } => {
                            let result = player.start_stream(url).await;
                            let _ = response.send(result);
                        }
                        AudioCommand::Stop { response } => {
                            player.stop().await;
                            let _ = response.send(());
                        }
                        AudioCommand::Pause => {
                            player.pause();
                        }
                        AudioCommand::Resume => {
                            player.resume();
                        }
                        AudioCommand::SetVolume(volume) => {
                            player.set_volume(volume);
                        }
                        AudioCommand::GetStats { response } => {
                            let stats = player.get_stats();
                            let _ = response.send(stats);
                        }
                        AudioCommand::SeekTo {
                            offset_secs,
                            response,
                        } => {
                            let result = player.seek_to(offset_secs).await;
                            let _ = response.send(result);
                        }
                        AudioCommand::GetBufferDepth { response } => {
                            let depth = player.get_buffer_depth().await;
                            let _ = response.send(depth.as_secs_f32());
                        }
                        AudioCommand::GetBufferTimeRange { response } => {
                            let range = player.get_buffer_time_range().await;
                            let _ = response.send(range);
                        }
                        AudioCommand::CanSeekTo {
                            offset_secs,
                            response,
                        } => {
                            let can_seek = player.can_seek_to(offset_secs).await;
                            let _ = response.send(can_seek);
                        }
                        AudioCommand::GetBufferDataFromOffset {
                            offset_secs,
                            response,
                        } => {
                            let data = player.get_buffer_data_from_offset(offset_secs).await;
                            let _ = response.send(data);
                        }
                        AudioCommand::IsAtLiveEdge { response } => {
                            let at_live = player.is_at_live_edge().await;
                            let _ = response.send(at_live);
                        }
                        AudioCommand::SetAtLiveEdge { at_live } => {
                            player.set_at_live_edge(at_live).await;
                        }
                        AudioCommand::Shutdown => {
                            player.stop().await;
                            break;
                        }
                    }
                }
            });
        });

        Ok(Self { command_tx })
    }

    // Create a new Send-safe gapless player (backwards compatibility)
    pub fn new() -> Result<Self> {
        Self::new_with_client(None)
    }

    // Start streaming from a URL
    pub async fn start_stream(&self, url: String) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx.send(AudioCommand::StartStream {
            url,
            response: response_tx,
        })?;
        response_rx.await?
    }

    // Stop streaming (only used internally for cleanup)
    #[allow(dead_code)]
    pub async fn stop(&self) {
        let (response_tx, response_rx) = oneshot::channel();
        let _ = self.command_tx.send(AudioCommand::Stop {
            response: response_tx,
        });
        let _ = response_rx.await;
    }

    // Pause playback
    pub fn pause(&self) {
        let _ = self.command_tx.send(AudioCommand::Pause);
    }

    // Resume playback
    pub fn resume(&self) {
        let _ = self.command_tx.send(AudioCommand::Resume);
    }

    // Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        let _ = self.command_tx.send(AudioCommand::SetVolume(volume));
    }

    // Get current streaming statistics (sync version for UI timer)
    #[allow(dead_code)] // Reserved for future UI stats display
    pub fn get_stats_sync(&self) -> StreamStats {
        // For now, return default stats since async get would block the UI
        // TODO: Implement stats caching on the send-safe wrapper side
        StreamStats::new()
    }

    // Seek to a specific time offset in the DVR buffer
    // Returns the actual seek position (clamped to valid buffer range)
    #[allow(dead_code)]
    pub async fn seek_to(&self, offset_secs: f32) -> Result<f32> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx.send(AudioCommand::SeekTo {
            offset_secs,
            response: response_tx,
        })?;
        response_rx.await?
    }

    // Get the time range of buffered content
    pub async fn get_buffer_time_range(&self) -> (f32, f32) {
        let (response_tx, response_rx) = oneshot::channel();
        let _ = self.command_tx.send(AudioCommand::GetBufferTimeRange {
            response: response_tx,
        });
        response_rx.await.unwrap_or((0.0, 0.0))
    }

    // Check if we can seek to a specific offset
    #[allow(dead_code)]
    pub async fn can_seek_to(&self, offset_secs: f32) -> bool {
        let (response_tx, response_rx) = oneshot::channel();
        let _ = self.command_tx.send(AudioCommand::CanSeekTo {
            offset_secs,
            response: response_tx,
        });
        response_rx.await.unwrap_or(false)
    }

    // Get buffered audio data from a specific offset
    // Used for DVR time-shifted playback
    pub async fn get_buffer_data_from_offset(&self, offset_secs: f32) -> Result<Vec<u8>> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(AudioCommand::GetBufferDataFromOffset {
                offset_secs,
                response: response_tx,
            })?;
        response_rx.await?
    }

    // Set whether we're at live edge or time-shifted
    pub async fn set_at_live_edge(&self, at_live: bool) {
        let _ = self
            .command_tx
            .send(AudioCommand::SetAtLiveEdge { at_live });
    }
}

// SAFETY: SendSafeGaplessPlayer is Send and Sync because:
// 1. command_tx is an mpsc::UnboundedSender which is Send + Sync
// 2. All non-Send data (OutputStream, GaplessPlayer) is owned by the dedicated audio thread
// 3. Communication happens only through Send types (channels and async tasks)
// 4. The audio thread has exclusive ownership of all rodio/symphonia resources
// 5. No shared mutable state exists between threads - all state is thread-local
unsafe impl Send for SendSafeGaplessPlayer {}
unsafe impl Sync for SendSafeGaplessPlayer {}

impl Drop for SendSafeGaplessPlayer {
    fn drop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
    }
}
