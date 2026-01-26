//! Audio playback system with mpv backend
//!
//! Provides audio playback control, track metadata extraction, and volume management.
//! Uses mpv as the backend player for its excellent terminal support and JSON IPC.
//!
//! NOTE: This module uses a static tokio runtime to avoid nested runtime issues.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Audio track metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<f64>, // Duration in seconds
    pub path: PathBuf,
}

impl TrackMetadata {
    /// Get display title (title if available, otherwise filename)
    pub fn display_title(&self) -> String {
        if let Some(ref title) = self.title {
            title.clone()
        } else {
            self.path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Unknown".to_string())
        }
    }

    /// Get display string (artist - title or just title)
    pub fn display_string(&self) -> String {
        match (&self.artist, &self.title) {
            (Some(artist), Some(title)) => format!("{} - {}", artist, title),
            (Some(artist), None) => format!("{} - {}", artist, self.display_title()),
            (None, Some(title)) => title.clone(),
            (None, None) => self.display_title(),
        }
    }
}

/// Playback state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Audio state - shared state for the player
#[derive(Debug, Clone)]
pub struct AudioState {
    pub current_track: Option<TrackMetadata>,
    pub playback_state: PlaybackState,
    pub volume: u8,    // 0-100
    pub position: f64, // Current position in seconds
    pub duration: f64, // Total duration in seconds
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            current_track: None,
            playback_state: PlaybackState::Stopped,
            volume: 80,
            position: 0.0,
            duration: 0.0,
        }
    }
}

/// MPV player implementation using synchronous commands
pub struct MpvPlayer {
    /// MPV process handle
    process: Arc<Mutex<Option<Child>>>,
    /// Current state
    state: Arc<Mutex<AudioState>>,
    /// Socket path for IPC
    socket_path: PathBuf,
    /// Available flag
    available: bool,
}

impl MpvPlayer {
    /// Create a new MPV player
    pub fn new() -> Self {
        let socket_path = std::env::temp_dir().join(format!("krusty-audio-{}", std::process::id()));

        Self {
            process: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(AudioState::default())),
            socket_path,
            available: false,
        }
    }

    /// Check if mpv is available
    pub fn check_available(&mut self) -> bool {
        self.available = which::which("mpv").is_ok();
        self.available
    }

    /// Start playback of a track
    #[allow(dead_code)]
    pub fn play(&mut self, path: PathBuf) -> Result<()> {
        // Extract metadata first (synchronously)
        let metadata = self.extract_metadata(&path);

        // Update state
        {
            let mut state = self.state.lock().unwrap();
            state.current_track = Some(metadata.clone());
            state.playback_state = PlaybackState::Playing;
            state.position = 0.0;
            state.duration = metadata.duration.unwrap_or(0.0);
        }

        // Kill any existing process
        self.stop();

        // Start mpv with audio-only mode and JSON IPC
        let mut cmd = Command::new("mpv");
        cmd.arg("--no-video")
            .arg("--no-terminal")
            .arg("--input-ipc-server")
            .arg(&self.socket_path)
            .arg("--volume")
            .arg("80")
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to start mpv for: {}", path.display()))?;

        let mut guard = self.process.lock().unwrap();
        *guard = Some(child);

        tracing::info!("Started playback: {}", metadata.display_string());

        Ok(())
    }

    /// Pause playback
    #[allow(dead_code)]
    pub fn pause(&self) {
        let _ = self.send_command("pause");
        let mut state = self.state.lock().unwrap();
        state.playback_state = PlaybackState::Paused;
    }

    /// Resume playback
    #[allow(dead_code)]
    pub fn resume(&self) {
        let _ = self.send_command("pause");
        let mut state = self.state.lock().unwrap();
        state.playback_state = PlaybackState::Playing;
    }

    /// Toggle play/pause
    pub fn toggle_pause(&self) {
        let _ = self.send_command("pause");
        let mut state = self.state.lock().unwrap();
        state.playback_state = match state.playback_state {
            PlaybackState::Playing => PlaybackState::Paused,
            PlaybackState::Paused => PlaybackState::Playing,
            PlaybackState::Stopped => PlaybackState::Stopped,
        };
    }

    /// Stop playback
    pub fn stop(&mut self) {
        // Send quit command first
        let _ = self.send_command("quit");

        // Kill process
        let mut guard = self.process.lock().unwrap();
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        // Clear socket path
        let _ = std::fs::remove_file(&self.socket_path);

        let mut state = self.state.lock().unwrap();
        state.playback_state = PlaybackState::Stopped;
        state.position = 0.0;
    }

    /// Set volume (0-100)
    #[allow(dead_code)]
    pub fn set_volume(&self, volume: u8) -> Result<()> {
        let volume = volume.clamp(0, 100);
        let cmd = format!("set volume {}", volume);
        self.send_command(&cmd)?;

        let mut state = self.state.lock().unwrap();
        state.volume = volume;
        Ok(())
    }

    /// Adjust volume by delta
    pub fn adjust_volume(&self, delta: i8) -> Result<u8> {
        let current = {
            let state = self.state.lock().unwrap();
            state.volume
        };
        let new_volume = (current as i16 + delta as i16).clamp(0, 100) as u8;
        self.set_volume(new_volume)?;
        Ok(new_volume)
    }

    /// Seek by seconds (relative)
    pub fn seek(&self, seconds: f64) {
        let cmd = format!("seek {}", seconds);
        let _ = self.send_command(&cmd);
    }

    /// Get current state
    pub fn state(&self) -> AudioState {
        self.state.lock().unwrap().clone()
    }

    /// Send command to mpv via socket
    fn send_command(&self, command: &str) -> Result<()> {
        use std::io::Write;

        // Try to connect to socket (non-blocking for quick response)
        let socket = std::os::unix::net::UnixStream::connect(&self.socket_path);

        if let Ok(mut socket) = socket {
            // Set a short read timeout
            socket.set_read_timeout(Some(std::time::Duration::from_millis(50)))?;

            let cmd = format!("{{\"command\": [\"{}\"]}}\n", command);
            socket.write_all(cmd.as_bytes())?;
            socket.flush()?;
        }

        Ok(())
    }

    /// Extract metadata from audio file (synchronous)
    #[allow(dead_code)]
    fn extract_metadata(&self, path: &PathBuf) -> TrackMetadata {
        let mut metadata = TrackMetadata {
            path: path.clone(),
            ..Default::default()
        };

        // Try to get duration using ffprobe
        if which::which("ffprobe").is_ok() {
            if let Ok(output) = Command::new("ffprobe")
                .arg("-v")
                .arg("error")
                .arg("-show_entries")
                .arg("format=duration")
                .arg("-of")
                .arg("default=noprint_wrappers=1:nokey=1")
                .arg(path)
                .output()
            {
                if output.status.success() {
                    let duration_str = String::from_utf8_lossy(&output.stdout);
                    if let Ok(d) = duration_str.trim().parse::<f64>() {
                        metadata.duration = Some(d);
                    }
                }
            }

            // Try to get tags
            if let Ok(output) = Command::new("ffprobe")
                .arg("-v")
                .arg("error")
                .arg("-show_entries")
                .arg("format_tags=title,artist,album")
                .arg("-of")
                .arg("json")
                .arg(path)
                .output()
            {
                if output.status.success() {
                    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                        if let Some(tags) = json.pointer("/format/tags").and_then(|t| t.as_object())
                        {
                            metadata.title = tags
                                .get("title")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            metadata.artist = tags
                                .get("artist")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            metadata.album = tags
                                .get("album")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                }
            }
        }

        metadata
    }
}

impl Default for MpvPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Global audio player instance (thread-safe)
#[derive(Clone)]
pub struct AudioPlayer {
    player: Arc<Mutex<MpvPlayer>>,
}

impl AudioPlayer {
    /// Create a new audio player
    pub fn new() -> Self {
        Self {
            player: Arc::new(Mutex::new(MpvPlayer::new())),
        }
    }

    /// Check if audio playback is available
    pub fn is_available(&self) -> bool {
        let mut player = self.player.lock().unwrap();
        player.check_available()
    }

    /// Play a track
    #[allow(dead_code)]
    pub fn play(&self, path: PathBuf) {
        let mut player = self.player.lock().unwrap();
        let _ = player.play(path);
    }

    /// Toggle play/pause
    pub fn toggle_pause(&self) {
        let player = self.player.lock().unwrap();
        player.toggle_pause();
    }

    /// Pause
    pub fn pause(&self) {
        let player = self.player.lock().unwrap();
        player.pause();
    }

    /// Resume
    #[allow(dead_code)]
    pub fn resume(&self) {
        let player = self.player.lock().unwrap();
        player.resume();
    }

    /// Stop playback
    pub fn stop(&self) {
        let mut player = self.player.lock().unwrap();
        player.stop();
    }

    /// Set volume (0-100)
    #[allow(dead_code)]
    pub fn set_volume(&self, volume: u8) {
        let player = self.player.lock().unwrap();
        let _ = player.set_volume(volume);
    }

    /// Adjust volume by delta
    pub fn adjust_volume(&self, delta: i8) -> u8 {
        let player = self.player.lock().unwrap();
        player.adjust_volume(delta).unwrap_or(80)
    }

    /// Seek by seconds (relative)
    pub fn seek(&self, seconds: f64) {
        let player = self.player.lock().unwrap();
        player.seek(seconds);
    }

    /// Get current state
    pub fn state(&self) -> AudioState {
        let player = self.player.lock().unwrap();
        player.state()
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}
