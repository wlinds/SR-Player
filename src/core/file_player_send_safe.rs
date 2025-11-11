// Send-safe wrapper for FilePlayer
//
// The FilePlayer contains rodio's OutputStream which has raw pointers (*mut ()) that aren't Send.
// Solution: Keep FilePlayer on a dedicated thread, communicate via channels.

use anyhow::Result;
use tokio::sync::{mpsc, oneshot};

// Commands that can be sent to the file player thread
#[derive(Debug)]
pub enum FilePlayerCommand {
    PlayFromBytesAtPosition {
        data: bytes::Bytes,
        position: f32,
        response: oneshot::Sender<Result<()>>,
    },
    Stop,
    Pause,
    Resume,
    SetVolume(f32),
    Shutdown,
}

// Send-safe wrapper for FilePlayer
pub struct SendSafeFilePlayer {
    command_tx: mpsc::UnboundedSender<FilePlayerCommand>,
}

impl SendSafeFilePlayer {
    // Create a new Send-safe file player
    pub fn new() -> Result<Self> {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel::<FilePlayerCommand>();

        // Spawn dedicated audio thread
        std::thread::spawn(move || {
            // Create tokio runtime for this thread
            let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

            rt.block_on(async {
                // Import inside the thread to keep it thread-local
                use crate::core::file_player::FilePlayer;

                let player = match FilePlayer::new() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("Failed to create FilePlayer: {}", e);
                        return;
                    }
                };

                // Process commands from UI thread
                while let Some(cmd) = command_rx.recv().await {
                    match cmd {
                        FilePlayerCommand::PlayFromBytesAtPosition {
                            data,
                            position,
                            response,
                        } => {
                            // Play from downloaded bytes at specific position
                            let result = player.play_from_bytes_at_position(data, position).await;
                            let _ = response.send(result);
                        }
                        FilePlayerCommand::Stop => {
                            player.stop().await;
                        }
                        FilePlayerCommand::Pause => {
                            player.pause();
                        }
                        FilePlayerCommand::Resume => {
                            player.resume();
                        }
                        FilePlayerCommand::SetVolume(volume) => {
                            player.set_volume(volume);
                        }
                        FilePlayerCommand::Shutdown => {
                            player.stop().await;
                            break;
                        }
                    }
                }
            });
        });

        Ok(Self { command_tx })
    }

    // Play from already-downloaded bytes at a specific position
    pub async fn play_from_bytes_at_position(
        &self,
        data: bytes::Bytes,
        position: f32,
    ) -> Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(FilePlayerCommand::PlayFromBytesAtPosition {
                data,
                position,
                response: response_tx,
            })?;
        response_rx.await?
    }

    // Stop playback
    pub fn stop(&self) {
        let _ = self.command_tx.send(FilePlayerCommand::Stop);
    }

    // Pause playback
    pub fn pause(&self) {
        let _ = self.command_tx.send(FilePlayerCommand::Pause);
    }

    // Resume playback
    pub fn resume(&self) {
        let _ = self.command_tx.send(FilePlayerCommand::Resume);
    }

    // Set volume (0.0 to 1.0)
    pub fn set_volume(&self, volume: f32) {
        let _ = self.command_tx.send(FilePlayerCommand::SetVolume(volume));
    }
}

// SAFETY: SendSafeFilePlayer is Send and Sync because:
// 1. command_tx is an mpsc::UnboundedSender which is Send + Sync
// 2. All non-Send data (OutputStream, FilePlayer) is owned by the dedicated audio thread
unsafe impl Send for SendSafeFilePlayer {}
unsafe impl Sync for SendSafeFilePlayer {}

impl Drop for SendSafeFilePlayer {
    fn drop(&mut self) {
        let _ = self.command_tx.send(FilePlayerCommand::Shutdown);
    }
}
