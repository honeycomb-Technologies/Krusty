//! RetroArch Plugin
//!
//! Runs libretro cores (game emulators) with Kitty graphics output.
//! Supports any libretro-compatible core (NES, SNES, GB, GBA, etc.)

use std::any::Any;
use std::os::unix::io::AsRawFd;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use parking_lot::Mutex;
use ratatui::{buffer::Buffer, layout::Rect};
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// RAII guard to suppress stdout/stderr during libretro core execution
/// This prevents core debug output from corrupting the terminal
struct SuppressStdio {
    stdout_fd: i32,
    stderr_fd: i32,
    stdout_backup: i32,
    stderr_backup: i32,
}

impl SuppressStdio {
    fn new() -> Self {
        // SAFETY: File descriptors are valid - stdout/stderr are obtained from Rust std.
        // dup/dup2/open/close are standard POSIX operations that are safe with valid fds.
        // The null check on devnull ensures we don't use an invalid fd.
        // Restoration in Drop maintains the invariant that stdout/stderr are valid.
        unsafe {
            let stdout_fd = std::io::stdout().as_raw_fd();
            let stderr_fd = std::io::stderr().as_raw_fd();

            // Backup original fds
            let stdout_backup = libc::dup(stdout_fd);
            let stderr_backup = libc::dup(stderr_fd);

            // Redirect to /dev/null
            let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if devnull >= 0 {
                libc::dup2(devnull, stdout_fd);
                libc::dup2(devnull, stderr_fd);
                libc::close(devnull);
            }

            Self {
                stdout_fd,
                stderr_fd,
                stdout_backup,
                stderr_backup,
            }
        }
    }
}

impl Drop for SuppressStdio {
    fn drop(&mut self) {
        // SAFETY: The backup fds were obtained from dup() in new() and are valid.
        // We check >= 0 before using them. dup2 and close are safe POSIX operations.
        // This restores stdout/stderr to their original state.
        unsafe {
            // Restore original fds
            if self.stdout_backup >= 0 {
                libc::dup2(self.stdout_backup, self.stdout_fd);
                libc::close(self.stdout_backup);
            }
            if self.stderr_backup >= 0 {
                libc::dup2(self.stderr_backup, self.stderr_fd);
                libc::close(self.stderr_backup);
            }
        }
    }
}

use super::libretro::{
    EnvironmentCmd, GameInfo, JoypadButton, LibRetroCore, PixelFormat, SystemAvInfo,
};
use super::{Plugin, PluginContext, PluginEventResult, PluginFrame, PluginRenderMode};

// ============================================================================
// STATE MACHINE
// ============================================================================

/// Main plugin states
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetroArchState {
    /// No game loaded - show menu
    Menu(MenuState),
    /// Game running
    Playing,
    /// In-game pause menu
    Paused,
}

impl Default for RetroArchState {
    fn default() -> Self {
        Self::Menu(MenuState::Main)
    }
}

/// Menu sub-states
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuState {
    /// Main menu: Load Game, Settings, Exit
    Main,
    /// Selecting a core
    CoreBrowser,
    /// Selecting a ROM
    RomBrowser,
}

/// Shared state between the plugin and libretro callbacks
struct SharedState {
    /// Current frame buffer (RGBA), Arc for zero-copy sharing
    frame_buffer: Arc<Vec<u8>>,
    /// Frame dimensions
    frame_width: u32,
    frame_height: u32,
    /// Pixel format from core
    pixel_format: PixelFormat,
    /// Frame ready flag
    frame_ready: bool,
    /// Frame counter for input timing
    frame_count: u64,
    /// Last frame each button was pressed
    button_press_frame: [u64; 16],
    /// Scratch buffer for video conversion (avoids allocation in video_refresh)
    scratch_buffer: Vec<u8>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            frame_buffer: Arc::new(Vec::new()),
            frame_width: 0,
            frame_height: 0,
            pixel_format: PixelFormat::XRGB8888,
            frame_ready: false,
            frame_count: 0,
            button_press_frame: [0; 16],
            scratch_buffer: Vec::with_capacity(256 * 256 * 4), // Common Game Boy size
        }
    }
}

// Global state for callbacks (libretro uses C callbacks without context)
static SHARED_STATE: Mutex<Option<Arc<Mutex<SharedState>>>> = Mutex::new(None);

/// Set the global shared state for callbacks
fn set_shared_state(state: Arc<Mutex<SharedState>>) {
    tracing::info!("set_shared_state: Setting global state");
    *SHARED_STATE.lock() = Some(state);
}

/// Clear the global shared state only if it matches the given state
/// This prevents a dropped plugin from clearing another plugin's state
fn clear_shared_state_if_owner(owner: &Arc<Mutex<SharedState>>) {
    let mut guard = SHARED_STATE.lock();
    if let Some(current) = guard.as_ref() {
        if Arc::ptr_eq(current, owner) {
            tracing::debug!("clear_shared_state_if_owner: Clearing state (is owner)");
            *guard = None;
        } else {
            tracing::debug!("clear_shared_state_if_owner: Not clearing (different owner)");
        }
    }
}

/// Get the shared state for callbacks
fn with_shared_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut SharedState) -> R,
{
    let guard = SHARED_STATE.lock();
    if guard.is_none() {
        tracing::warn!("with_shared_state: SHARED_STATE is None!");
        return None;
    }
    guard.as_ref().map(|state| f(&mut state.lock()))
}

// LibRetro callbacks
extern "C" fn video_refresh(data: *const std::ffi::c_void, width: u32, height: u32, pitch: usize) {
    if data.is_null() || width == 0 || height == 0 || pitch == 0 {
        tracing::trace!(
            "video_refresh: skipping invalid frame (null={}, {}x{}, pitch={})",
            data.is_null(),
            width,
            height,
            pitch
        );
        return; // Invalid or duplicate frame, skip
    }

    with_shared_state(|state| {
        // Log first few frames at info level for debugging
        if state.frame_count < 5 {
            tracing::info!(
                "video_refresh: frame {}x{} pitch={} format={:?}",
                width,
                height,
                pitch,
                state.pixel_format
            );
        }

        let bytes_per_pixel = match state.pixel_format {
            PixelFormat::XRGB8888 => 4,
            PixelFormat::RGB565 | PixelFormat::RGB1555 => 2,
        };

        // Sanity check dimensions
        if width > 4096 || height > 4096 {
            return;
        }

        // Calculate total size and use scratch buffer to avoid allocation
        let rgba_size = (width as usize) * (height as usize) * 4;
        state.scratch_buffer.clear();
        state.scratch_buffer.resize(rgba_size, 0);

        // Convert to RGBA based on pixel format
        // SAFETY: The data pointer comes from the libretro core's video_refresh callback.
        // The pitch and height are provided by the core and validated (non-zero, < 4096).
        // The libretro API guarantees the buffer is valid for pitch * height bytes.
        let src = unsafe { std::slice::from_raw_parts(data as *const u8, pitch * height as usize) };

        for y in 0..height as usize {
            let row_start = y * pitch;
            let dst_row_start = y * (width as usize) * 4;

            for x in 0..width as usize {
                let pixel_offset = row_start + x * bytes_per_pixel;
                let dst_offset = dst_row_start + x * 4;

                // Bounds check
                if pixel_offset + bytes_per_pixel > src.len() {
                    continue;
                }

                let (r, g, b) = match state.pixel_format {
                    PixelFormat::XRGB8888 => {
                        let pixel = u32::from_ne_bytes([
                            src[pixel_offset],
                            src[pixel_offset + 1],
                            src[pixel_offset + 2],
                            src[pixel_offset + 3],
                        ]);
                        (
                            ((pixel >> 16) & 0xFF) as u8,
                            ((pixel >> 8) & 0xFF) as u8,
                            (pixel & 0xFF) as u8,
                        )
                    }
                    PixelFormat::RGB565 => {
                        let pixel = u16::from_ne_bytes([src[pixel_offset], src[pixel_offset + 1]]);
                        (
                            ((pixel >> 11) as u8) << 3,
                            (((pixel >> 5) & 0x3F) as u8) << 2,
                            ((pixel & 0x1F) as u8) << 3,
                        )
                    }
                    PixelFormat::RGB1555 => {
                        let pixel = u16::from_ne_bytes([src[pixel_offset], src[pixel_offset + 1]]);
                        (
                            ((pixel >> 10) as u8 & 0x1F) << 3,
                            ((pixel >> 5) as u8 & 0x1F) << 3,
                            (pixel as u8 & 0x1F) << 3,
                        )
                    }
                };

                state.scratch_buffer[dst_offset] = r;
                state.scratch_buffer[dst_offset + 1] = g;
                state.scratch_buffer[dst_offset + 2] = b;
                state.scratch_buffer[dst_offset + 3] = 255;
            }
        }

        // Swap scratch buffer into frame buffer wrapped in Arc
        let new_buffer = std::mem::take(&mut state.scratch_buffer);
        state.frame_buffer = Arc::new(new_buffer);
        // Pre-allocate scratch buffer for next frame
        state.scratch_buffer = Vec::with_capacity(rgba_size);

        state.frame_width = width;
        state.frame_height = height;
        state.frame_ready = true;
        // Log first few times to confirm this code runs
        if state.frame_count < 10 {
            tracing::info!(
                "video_refresh: set frame_ready=true, buf_len={}",
                state.frame_buffer.len()
            );
        }
    });
}

extern "C" fn audio_sample(_left: i16, _right: i16) {
    // Audio not implemented yet
}

extern "C" fn audio_sample_batch(_data: *const i16, frames: usize) -> usize {
    frames // Pretend we consumed all samples
}

extern "C" fn input_poll() {
    // Input state is updated via handle_event
}

extern "C" fn input_state(_port: u32, device: u32, _index: u32, id: u32) -> i16 {
    if device != 1 || id > 15 {
        // Only joypad supported, valid button IDs are 0-15
        return 0;
    }

    with_shared_state(|state| {
        // Button is considered pressed if it was pressed within the last few frames
        // This provides a "sticky" effect that works well with terminal key repeat
        let press_frame = state.button_press_frame[id as usize];
        let frames_since_press = state.frame_count.saturating_sub(press_frame);

        // Button stays pressed for ~8 frames (~133ms at 60fps)
        // This smooths out the terminal's key repeat timing
        if frames_since_press < 8 {
            1
        } else {
            0
        }
    })
    .unwrap_or(0)
}

// Static paths for environment callbacks (avoid repeated allocations)
static SYSTEM_DIR: std::sync::LazyLock<CString> = std::sync::LazyLock::new(|| {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = home.join(".config/krusty/retroarch/system");
    let _ = std::fs::create_dir_all(&dir);
    CString::new(dir.to_string_lossy().as_ref()).unwrap_or_default()
});

static SAVE_DIR: std::sync::LazyLock<CString> = std::sync::LazyLock::new(|| {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = home.join(".config/krusty/retroarch/saves");
    let _ = std::fs::create_dir_all(&dir);
    CString::new(dir.to_string_lossy().as_ref()).unwrap_or_default()
});

extern "C" fn environment(cmd: u32, data: *mut std::ffi::c_void) -> bool {
    match cmd {
        cmd if cmd == EnvironmentCmd::SetPixelFormat as u32 => {
            if data.is_null() {
                return false;
            }
            // SAFETY: Null check above ensures data is valid. The libretro API
            // guarantees data points to a u32 for SET_PIXEL_FORMAT command.
            let format = unsafe { *(data as *const u32) };
            with_shared_state(|state| {
                state.pixel_format = match format {
                    0 => {
                        tracing::debug!("RetroArch: Pixel format set to RGB1555");
                        PixelFormat::RGB1555
                    }
                    1 => {
                        tracing::debug!("RetroArch: Pixel format set to XRGB8888");
                        PixelFormat::XRGB8888
                    }
                    2 => {
                        tracing::debug!("RetroArch: Pixel format set to RGB565");
                        PixelFormat::RGB565
                    }
                    _ => return false,
                };
                true
            })
            .unwrap_or(false)
        }
        cmd if cmd == EnvironmentCmd::GetSystemDirectory as u32 => {
            if data.is_null() {
                return false;
            }
            // SAFETY: Null check above ensures data is valid. SYSTEM_DIR is a static
            // CString that lives for the program lifetime. The libretro API expects
            // a pointer to a C string pointer for GET_SYSTEM_DIRECTORY.
            unsafe {
                *(data as *mut *const i8) = SYSTEM_DIR.as_ptr();
            }
            true
        }
        cmd if cmd == EnvironmentCmd::GetSaveDirectory as u32 => {
            if data.is_null() {
                return false;
            }
            // SAFETY: Same as GetSystemDirectory - SAVE_DIR is a static CString.
            unsafe {
                *(data as *mut *const i8) = SAVE_DIR.as_ptr();
            }
            true
        }
        cmd if cmd == EnvironmentCmd::GetCanDupe as u32 => {
            if !data.is_null() {
                // SAFETY: Null check above ensures data is valid. The libretro API
                // expects a pointer to bool for GET_CAN_DUPE.
                unsafe {
                    *(data as *mut bool) = true;
                }
            }
            true
        }
        cmd if cmd == EnvironmentCmd::GetLogInterface as u32 => {
            // Decline log interface to suppress core logging
            false
        }
        _ => false, // Unhandled
    }
}

/// RetroArch plugin for running libretro cores
pub struct RetroArchPlugin {
    /// Loaded core (if any)
    core: Option<LibRetroCore>,
    /// Path to loaded ROM
    rom_path: Option<PathBuf>,
    /// Core name
    core_name: String,
    /// Shared state with callbacks
    shared_state: Arc<Mutex<SharedState>>,
    /// Whether game is running
    running: bool,
    /// AV info from core
    av_info: Option<SystemAvInfo>,
    /// Error message to display
    error: Option<String>,

    // State machine
    /// Current plugin state (Menu, Playing, Paused)
    plugin_state: RetroArchState,

    // Menu state
    /// Available cores
    cores: Vec<PathBuf>,
    /// Available ROMs in current directory
    roms: Vec<PathBuf>,
    /// Current directory for ROM browsing
    current_rom_dir: PathBuf,
    /// Selected core for ROM loading
    selected_core: Option<PathBuf>,
    /// Selected menu index
    menu_index: usize,
    /// Scroll offset for long lists
    scroll_offset: usize,
    /// Selected pause menu option
    pause_index: usize,
}

/// Krusty RetroArch directories
struct KrustyDirs {
    system: PathBuf,
    saves: PathBuf,
    states: PathBuf,
    roms: PathBuf,
}

impl KrustyDirs {
    fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let base = home.join(".config/krusty/retroarch");

        let dirs = Self {
            system: base.join("system"),
            saves: base.join("saves"),
            states: base.join("states"),
            roms: base.join("roms"),
        };

        // Create directories if they don't exist
        let _ = std::fs::create_dir_all(&dirs.system);
        let _ = std::fs::create_dir_all(&dirs.saves);
        let _ = std::fs::create_dir_all(&dirs.states);
        let _ = std::fs::create_dir_all(&dirs.roms);

        dirs
    }
}

static KRUSTY_DIRS: std::sync::LazyLock<KrustyDirs> = std::sync::LazyLock::new(KrustyDirs::new);

impl RetroArchPlugin {
    pub fn new() -> Self {
        Self {
            core: None,
            rom_path: None,
            core_name: "No core loaded".to_string(),
            shared_state: Arc::new(Mutex::new(SharedState::default())),
            running: false,
            av_info: None,
            error: None,

            // State machine - start in menu
            plugin_state: RetroArchState::default(),

            // Menu state
            cores: Vec::new(),
            roms: Vec::new(),
            current_rom_dir: KRUSTY_DIRS.roms.clone(),
            selected_core: None,
            menu_index: 0,
            scroll_offset: 0,
            pause_index: 0,
        }
    }

    /// Load a libretro core
    pub fn load_core(&mut self, core_path: &Path) -> Result<(), String> {
        // Unload existing core
        self.unload();

        // Suppress stdout/stderr during core loading
        let _guard = SuppressStdio::new();

        // SAFETY: LibRetroCore::load performs dynamic library loading via libloading.
        // The path is validated to exist by the caller. The core is a libretro-compliant
        // shared library that exports the required symbols. Any loading failures are
        // returned as errors rather than causing undefined behavior.
        let core = unsafe { LibRetroCore::load(core_path)? };

        // Set up callbacks
        set_shared_state(self.shared_state.clone());
        (core.retro_set_environment)(environment);
        (core.retro_set_video_refresh)(video_refresh);
        (core.retro_set_audio_sample)(audio_sample);
        (core.retro_set_audio_sample_batch)(audio_sample_batch);
        (core.retro_set_input_poll)(input_poll);
        (core.retro_set_input_state)(input_state);

        // Initialize
        (core.retro_init)();

        self.core_name = format!("{} {}", core.name(), core.version());
        self.core = Some(core);
        self.error = None;

        // Verify state is set (log after SuppressStdio guard is dropped)
        drop(_guard);
        let state_set = SHARED_STATE.lock().is_some();
        tracing::info!("load_core: Complete, SHARED_STATE is_some={}", state_set);

        Ok(())
    }

    /// Load a ROM file
    pub fn load_rom(&mut self, rom_path: &Path) -> Result<(), String> {
        let core = self.core.as_ref().ok_or("No core loaded")?;

        // Read ROM file
        let rom_data = std::fs::read(rom_path).map_err(|e| format!("Failed to read ROM: {}", e))?;

        let path_cstr =
            CString::new(rom_path.to_string_lossy().as_ref()).map_err(|e| e.to_string())?;

        let game_info = GameInfo {
            path: path_cstr.as_ptr(),
            data: rom_data.as_ptr() as *const std::ffi::c_void,
            size: rom_data.len(),
            meta: std::ptr::null(),
        };

        // Suppress stdout/stderr during ROM loading
        let _guard = SuppressStdio::new();

        let loaded = (core.retro_load_game)(&game_info);
        if !loaded {
            return Err("Core failed to load ROM".to_string());
        }

        // Get AV info
        let mut av_info = SystemAvInfo {
            geometry: super::libretro::GameGeometry {
                base_width: 0,
                base_height: 0,
                max_width: 0,
                max_height: 0,
                aspect_ratio: 0.0,
            },
            timing: super::libretro::SystemTiming {
                fps: 60.0,
                sample_rate: 44100.0,
            },
        };
        (core.retro_get_system_av_info)(&mut av_info);
        self.av_info = Some(av_info);

        // Set controller
        (core.retro_set_controller_port_device)(0, 1); // Port 0, Joypad

        self.rom_path = Some(rom_path.to_path_buf());
        self.running = true;
        self.error = None;

        // Load SRAM if available (battery saves)
        if let Err(e) = self.load_sram() {
            tracing::warn!("Failed to load SRAM: {}", e);
        }

        Ok(())
    }

    /// Unload core and ROM
    pub fn unload(&mut self) {
        if let Some(core) = self.core.take() {
            if self.rom_path.is_some() {
                (core.retro_unload_game)();
            }
            (core.retro_deinit)();
        }
        // Only clear global state if we own it (prevents dropped plugins from
        // clearing another plugin's state)
        clear_shared_state_if_owner(&self.shared_state);
        self.rom_path = None;
        self.running = false;
        self.av_info = None;
        self.core_name = "No core loaded".to_string();
    }

    /// Run one frame
    pub fn run_frame(&mut self) {
        if let Some(core) = &self.core {
            if self.running {
                // Don't suppress stdout during frame execution - too expensive (60fps)
                // Only suppress during initial load when cores print debug info
                (core.retro_run)();
            }
        }
    }

    /// Press a button by its libretro button ID (called from gamepad handler)
    pub fn press_button(&mut self, button_id: u8) {
        if button_id < 16 {
            let mut state = self.shared_state.lock();
            state.button_press_frame[button_id as usize] = state.frame_count;
        }
    }

    /// Map keyboard to joypad
    fn key_to_button(key: KeyCode, _modifiers: KeyModifiers) -> Option<JoypadButton> {
        // Default mapping (can be customized later)
        match key {
            KeyCode::Char('x') | KeyCode::Char('X') => Some(JoypadButton::A),
            KeyCode::Char('z') | KeyCode::Char('Z') => Some(JoypadButton::B),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(JoypadButton::Y),
            KeyCode::Char('s') | KeyCode::Char('S') => Some(JoypadButton::X),
            KeyCode::Enter => Some(JoypadButton::Start),
            KeyCode::Backspace => Some(JoypadButton::Select),
            KeyCode::Up => Some(JoypadButton::Up),
            KeyCode::Down => Some(JoypadButton::Down),
            KeyCode::Left => Some(JoypadButton::Left),
            KeyCode::Right => Some(JoypadButton::Right),
            KeyCode::Char('q') | KeyCode::Char('Q') => Some(JoypadButton::L),
            KeyCode::Char('w') | KeyCode::Char('W') => Some(JoypadButton::R),
            _ => None,
        }
    }

    // =========================================================================
    // MENU METHODS
    // =========================================================================

    /// Scan for available libretro cores
    fn scan_cores(&mut self) {
        self.cores.clear();
        let core_dir = PathBuf::from("/usr/lib/libretro");
        if let Ok(entries) = std::fs::read_dir(&core_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "so").unwrap_or(false) {
                    self.cores.push(path);
                }
            }
        }
        self.cores.sort();
    }

    /// Scan for ROMs in current directory
    fn scan_roms(&mut self) {
        self.roms.clear();

        // Add parent directory entry
        if self.current_rom_dir.parent().is_some() {
            self.roms.push(PathBuf::from(".."));
        }

        if let Ok(entries) = std::fs::read_dir(&self.current_rom_dir) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();

            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().map(|n| n.to_string_lossy().to_string());

                // Skip hidden files
                if name.as_ref().map(|n| n.starts_with('.')).unwrap_or(false) {
                    continue;
                }

                if path.is_dir() {
                    dirs.push(path);
                } else if Self::is_rom_file(&path, self.selected_core.as_deref()) {
                    files.push(path);
                }
            }

            dirs.sort();
            files.sort();

            self.roms.extend(dirs);
            self.roms.extend(files);
        }
    }

    /// Check if a file is a ROM based on extension and selected core
    fn is_rom_file(path: &Path, selected_core: Option<&Path>) -> bool {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        // Get core-specific extensions if we know the core
        if let Some(core_path) = selected_core {
            let core_name = core_path
                .file_stem()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            // Match core to supported extensions
            let extensions: &[&str] = match core_name.as_str() {
                s if s.contains("gambatte") => &["gb", "gbc"],
                s if s.contains("mgba") || s.contains("vba") => &["gba", "gb", "gbc"],
                s if s.contains("snes9x") || s.contains("bsnes") => &["sfc", "smc"],
                s if s.contains("nestopia") || s.contains("fceumm") => &["nes"],
                s if s.contains("genesis") || s.contains("picodrive") => &["md", "gen", "smd"],
                s if s.contains("mupen") || s.contains("parallel") => &["n64", "z64", "v64"],
                s if s.contains("pcsx") || s.contains("beetle_psx") => &["bin", "cue", "iso"],
                s if s.contains("desmume") || s.contains("melonds") => &["nds"],
                s if s.contains("ppsspp") => &["iso", "cso"],
                _ => &[
                    "gb", "gbc", "gba", "nes", "sfc", "smc", "md", "gen", "n64", "nds",
                ],
            };

            return extensions.contains(&ext.as_str());
        }

        // Default: accept common ROM extensions
        matches!(
            ext.as_str(),
            "gb" | "gbc"
                | "gba"
                | "nes"
                | "sfc"
                | "smc"
                | "md"
                | "gen"
                | "smd"
                | "n64"
                | "z64"
                | "nds"
                | "iso"
                | "cso"
                | "bin"
                | "cue"
        )
    }

    /// Exit current ROM and return to menu
    pub fn exit_rom(&mut self) {
        // Save SRAM before unloading
        if let Err(e) = self.save_sram() {
            tracing::warn!("Failed to save SRAM on exit: {}", e);
        }
        self.unload();
        self.plugin_state = RetroArchState::Menu(MenuState::Main);
        self.menu_index = 0;
        self.scroll_offset = 0;
    }

    // =========================================================================
    // SAVE STATE METHODS
    // =========================================================================

    /// Get the save state path for a given slot
    fn state_path(&self, slot: u8) -> Option<PathBuf> {
        let rom_name = self.rom_path.as_ref()?.file_stem()?.to_string_lossy();
        let filename = if slot == 0 {
            format!("{}.state", rom_name)
        } else {
            format!("{}.state{}", rom_name, slot)
        };
        Some(KRUSTY_DIRS.states.join(filename))
    }

    /// Save state to a slot (0 = quick save)
    pub fn save_state(&mut self, slot: u8) -> Result<(), String> {
        let core = self.core.as_ref().ok_or("No core loaded")?;
        let path = self.state_path(slot).ok_or("No ROM loaded")?;

        let size = (core.retro_serialize_size)();
        if size == 0 {
            return Err("Core does not support save states".to_string());
        }

        let mut data = vec![0u8; size];
        let success = (core.retro_serialize)(data.as_mut_ptr() as *mut std::ffi::c_void, size);
        if !success {
            return Err("Failed to serialize state".to_string());
        }

        std::fs::write(&path, &data).map_err(|e| format!("Failed to write state: {}", e))?;
        tracing::info!("Saved state to {:?} ({} bytes)", path, size);
        Ok(())
    }

    /// Load state from a slot (0 = quick save)
    pub fn load_state(&mut self, slot: u8) -> Result<(), String> {
        let core = self.core.as_ref().ok_or("No core loaded")?;
        let path = self.state_path(slot).ok_or("No ROM loaded")?;

        if !path.exists() {
            return Err(format!("No save state in slot {}", slot));
        }

        let data = std::fs::read(&path).map_err(|e| format!("Failed to read state: {}", e))?;
        let success =
            (core.retro_unserialize)(data.as_ptr() as *const std::ffi::c_void, data.len());
        if !success {
            return Err("Failed to deserialize state".to_string());
        }

        tracing::info!("Loaded state from {:?} ({} bytes)", path, data.len());
        Ok(())
    }

    // =========================================================================
    // SRAM METHODS
    // =========================================================================

    /// Get the SRAM save path for current ROM
    fn sram_path(&self) -> Option<PathBuf> {
        let rom_name = self.rom_path.as_ref()?.file_stem()?.to_string_lossy();
        Some(KRUSTY_DIRS.saves.join(format!("{}.srm", rom_name)))
    }

    /// Save SRAM (battery save) to disk
    pub fn save_sram(&self) -> Result<(), String> {
        use super::libretro::RETRO_MEMORY_SAVE_RAM;

        let core = self.core.as_ref().ok_or("No core loaded")?;
        let path = self.sram_path().ok_or("No ROM loaded")?;

        let size = (core.retro_get_memory_size)(RETRO_MEMORY_SAVE_RAM);
        if size == 0 {
            return Ok(()); // No SRAM for this game
        }

        let data_ptr = (core.retro_get_memory_data)(RETRO_MEMORY_SAVE_RAM);
        if data_ptr.is_null() {
            return Err("Failed to get SRAM data pointer".to_string());
        }

        // SAFETY: The null check above ensures data_ptr is valid. The size comes from
        // retro_get_memory_size which returns the valid memory region size. The libretro
        // API guarantees the memory region is valid for the returned size.
        let data = unsafe { std::slice::from_raw_parts(data_ptr as *const u8, size) };
        std::fs::write(&path, data).map_err(|e| format!("Failed to write SRAM: {}", e))?;
        tracing::info!("Saved SRAM to {:?} ({} bytes)", path, size);
        Ok(())
    }

    /// Load SRAM (battery save) from disk
    pub fn load_sram(&self) -> Result<(), String> {
        use super::libretro::RETRO_MEMORY_SAVE_RAM;

        let core = self.core.as_ref().ok_or("No core loaded")?;
        let path = self.sram_path().ok_or("No ROM loaded")?;

        if !path.exists() {
            return Ok(()); // No save file yet
        }

        let size = (core.retro_get_memory_size)(RETRO_MEMORY_SAVE_RAM);
        if size == 0 {
            return Ok(()); // No SRAM for this game
        }

        let data_ptr = (core.retro_get_memory_data)(RETRO_MEMORY_SAVE_RAM);
        if data_ptr.is_null() {
            return Err("Failed to get SRAM data pointer".to_string());
        }

        let file_data = std::fs::read(&path).map_err(|e| format!("Failed to read SRAM: {}", e))?;
        let copy_size = file_data.len().min(size);

        // SAFETY: data_ptr is validated non-null above. size comes from retro_get_memory_size.
        // copy_size is min(file_data.len(), size) ensuring we don't write past either buffer.
        // The source (file_data) is a valid Rust Vec. The destination is a libretro memory
        // region guaranteed valid for 'size' bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(file_data.as_ptr(), data_ptr as *mut u8, copy_size);
        }

        tracing::info!("Loaded SRAM from {:?} ({} bytes)", path, copy_size);
        Ok(())
    }

    /// Get the number of items in current menu list
    fn menu_list_len(&self) -> usize {
        match &self.plugin_state {
            RetroArchState::Menu(MenuState::Main) => 2, // Load Game, Settings (future)
            RetroArchState::Menu(MenuState::CoreBrowser) => self.cores.len(),
            RetroArchState::Menu(MenuState::RomBrowser) => self.roms.len(),
            RetroArchState::Paused => 5, // Resume, Save, Load, Reset, Exit
            RetroArchState::Playing => 0,
        }
    }

    /// Handle menu navigation (Up/Down)
    fn menu_navigate(&mut self, delta: i32) {
        let len = self.menu_list_len();
        if len == 0 {
            return;
        }

        let new_index = if delta > 0 {
            (self.menu_index + delta as usize).min(len - 1)
        } else {
            self.menu_index.saturating_sub((-delta) as usize)
        };

        self.menu_index = new_index;

        // Ensure visible (scroll if needed)
        const VISIBLE_HEIGHT: usize = 10;
        if self.menu_index < self.scroll_offset {
            self.scroll_offset = self.menu_index;
        } else if self.menu_index >= self.scroll_offset + VISIBLE_HEIGHT {
            self.scroll_offset = self.menu_index - VISIBLE_HEIGHT + 1;
        }
    }

    /// Handle menu selection (Enter)
    fn menu_select(&mut self) {
        match &self.plugin_state {
            RetroArchState::Menu(MenuState::Main) => {
                match self.menu_index {
                    0 => {
                        // Load Game - go to core browser
                        self.plugin_state = RetroArchState::Menu(MenuState::CoreBrowser);
                        self.scan_cores();
                        self.menu_index = 0;
                        self.scroll_offset = 0;
                    }
                    1 => {
                        // Settings - not implemented yet
                    }
                    _ => {}
                }
            }
            RetroArchState::Menu(MenuState::CoreBrowser) => {
                if let Some(core) = self.cores.get(self.menu_index).cloned() {
                    self.selected_core = Some(core);
                    self.plugin_state = RetroArchState::Menu(MenuState::RomBrowser);
                    self.scan_roms();
                    self.menu_index = 0;
                    self.scroll_offset = 0;
                }
            }
            RetroArchState::Menu(MenuState::RomBrowser) => {
                if let Some(path) = self.roms.get(self.menu_index).cloned() {
                    if path.as_os_str() == ".." {
                        // Go up a directory
                        if let Some(parent) = self.current_rom_dir.parent() {
                            self.current_rom_dir = parent.to_path_buf();
                            self.scan_roms();
                            self.menu_index = 0;
                            self.scroll_offset = 0;
                        }
                    } else if path.is_dir() {
                        // Enter directory
                        self.current_rom_dir = path;
                        self.scan_roms();
                        self.menu_index = 0;
                        self.scroll_offset = 0;
                    } else if let Some(core_path) = &self.selected_core.clone() {
                        // ROM selected - load game
                        if let Err(e) = self.load_core(core_path) {
                            self.error = Some(format!("Core load failed: {}", e));
                        } else if let Err(e) = self.load_rom(&path) {
                            self.error = Some(format!("ROM load failed: {}", e));
                        } else {
                            self.plugin_state = RetroArchState::Playing;
                        }
                    }
                }
            }
            RetroArchState::Paused => {
                match self.pause_index {
                    0 => {
                        // Resume
                        self.plugin_state = RetroArchState::Playing;
                    }
                    1 => {
                        // Save State (slot 0 = quick save)
                        match self.save_state(0) {
                            Ok(()) => {
                                self.error = None;
                                self.plugin_state = RetroArchState::Playing;
                            }
                            Err(e) => {
                                self.error = Some(format!("Save failed: {}", e));
                            }
                        }
                    }
                    2 => {
                        // Load State (slot 0 = quick save)
                        match self.load_state(0) {
                            Ok(()) => {
                                self.error = None;
                                self.plugin_state = RetroArchState::Playing;
                            }
                            Err(e) => {
                                self.error = Some(format!("Load failed: {}", e));
                            }
                        }
                    }
                    3 => {
                        // Reset
                        if let Some(core) = &self.core {
                            (core.retro_reset)();
                        }
                        self.plugin_state = RetroArchState::Playing;
                    }
                    4 => {
                        // Exit ROM
                        self.exit_rom();
                    }
                    _ => {}
                }
            }
            RetroArchState::Playing => {}
        }
    }

    /// Handle menu back (Esc/Backspace)
    fn menu_back(&mut self) -> bool {
        match &self.plugin_state {
            RetroArchState::Menu(MenuState::Main) => false, // Can't go back from main
            RetroArchState::Menu(MenuState::CoreBrowser) => {
                self.plugin_state = RetroArchState::Menu(MenuState::Main);
                self.menu_index = 0;
                self.scroll_offset = 0;
                true
            }
            RetroArchState::Menu(MenuState::RomBrowser) => {
                self.plugin_state = RetroArchState::Menu(MenuState::CoreBrowser);
                self.menu_index = 0;
                self.scroll_offset = 0;
                true
            }
            RetroArchState::Paused => {
                // Resume game on Esc from pause menu
                self.plugin_state = RetroArchState::Playing;
                true
            }
            RetroArchState::Playing => false,
        }
    }

    // =========================================================================
    // RENDERING METHODS
    // =========================================================================

    /// Render menu screen
    fn render_menu(
        &self,
        area: Rect,
        buf: &mut Buffer,
        ctx: &PluginContext,
        menu_state: &MenuState,
    ) {
        // Draw title
        let title = match menu_state {
            MenuState::Main => "RetroArch",
            MenuState::CoreBrowser => "Select Core",
            MenuState::RomBrowser => "Select ROM",
        };

        let title_y = area.y + 1;
        let title_x = area.x + (area.width.saturating_sub(title.len() as u16)) / 2;
        for (i, ch) in title.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((title_x + i as u16, title_y)) {
                cell.set_char(ch);
                cell.set_fg(ctx.theme.accent_color);
            }
        }

        // Draw separator
        let sep_y = title_y + 1;
        for x in area.x + 2..area.x + area.width - 2 {
            if let Some(cell) = buf.cell_mut((x, sep_y)) {
                cell.set_char('─');
                cell.set_fg(ctx.theme.border_color);
            }
        }

        // Draw error if any
        if let Some(error) = &self.error {
            let err_y = sep_y + 1;
            let err_text = format!("Error: {}", error);
            let err_x = area.x + 2;
            for (i, ch) in err_text.chars().take((area.width - 4) as usize).enumerate() {
                if let Some(cell) = buf.cell_mut((err_x + i as u16, err_y)) {
                    cell.set_char(ch);
                    cell.set_fg(ctx.theme.error_color);
                }
            }
        }

        // Draw menu items
        let content_y = sep_y + if self.error.is_some() { 3 } else { 2 };
        let visible_height = (area.height.saturating_sub(content_y - area.y + 3)) as usize;

        match menu_state {
            MenuState::Main => {
                let items = ["Load Game", "Settings"];
                for (i, item) in items.iter().enumerate() {
                    let y = content_y + i as u16;
                    if y >= area.y + area.height - 2 {
                        break;
                    }

                    let selected = i == self.menu_index;
                    let prefix = if selected { "▶ " } else { "  " };
                    let text = format!("{}{}", prefix, item);
                    let x = area.x + 4;

                    for (j, ch) in text.chars().enumerate() {
                        if let Some(cell) = buf.cell_mut((x + j as u16, y)) {
                            cell.set_char(ch);
                            cell.set_fg(if selected {
                                ctx.theme.accent_color
                            } else {
                                ctx.theme.text_color
                            });
                        }
                    }
                }
            }
            MenuState::CoreBrowser => {
                self.render_file_list(&self.cores, area, buf, ctx, content_y, visible_height, true);
            }
            MenuState::RomBrowser => {
                // Show current directory
                let dir_text = self.current_rom_dir.display().to_string();
                let dir_x = area.x + 2;
                let dir_y = content_y;
                for (i, ch) in dir_text.chars().take((area.width - 4) as usize).enumerate() {
                    if let Some(cell) = buf.cell_mut((dir_x + i as u16, dir_y)) {
                        cell.set_char(ch);
                        cell.set_fg(ctx.theme.dim_color);
                    }
                }

                self.render_file_list(
                    &self.roms,
                    area,
                    buf,
                    ctx,
                    content_y + 2,
                    visible_height.saturating_sub(2),
                    false,
                );
            }
        }

        // Draw footer
        let footer_y = area.y + area.height - 2;
        let footer = match menu_state {
            MenuState::Main => "↑↓: Navigate  Enter: Select",
            MenuState::CoreBrowser | MenuState::RomBrowser => {
                "↑↓: Navigate  Enter: Select  Esc: Back"
            }
        };
        let footer_x = area.x + (area.width.saturating_sub(footer.len() as u16)) / 2;
        for (i, ch) in footer.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((footer_x + i as u16, footer_y)) {
                cell.set_char(ch);
                cell.set_fg(ctx.theme.dim_color);
            }
        }
    }

    /// Render file list (cores or ROMs)
    fn render_file_list(
        &self,
        items: &[PathBuf],
        area: Rect,
        buf: &mut Buffer,
        ctx: &PluginContext,
        start_y: u16,
        visible_height: usize,
        show_full_name: bool,
    ) {
        if items.is_empty() {
            let msg = "No items found";
            let x = area.x + (area.width.saturating_sub(msg.len() as u16)) / 2;
            for (i, ch) in msg.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x + i as u16, start_y)) {
                    cell.set_char(ch);
                    cell.set_fg(ctx.theme.dim_color);
                }
            }
            return;
        }

        // Scroll indicator up
        if self.scroll_offset > 0 {
            let indicator = format!("▲ {} more", self.scroll_offset);
            let x = area.x + 4;
            for (i, ch) in indicator.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x + i as u16, start_y)) {
                    cell.set_char(ch);
                    cell.set_fg(ctx.theme.dim_color);
                }
            }
        }

        // Visible items
        let item_start_y = start_y + if self.scroll_offset > 0 { 1 } else { 0 };
        let visible_end = (self.scroll_offset + visible_height).min(items.len());

        for (idx, item) in items
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(visible_end - self.scroll_offset)
        {
            let y = item_start_y + (idx - self.scroll_offset) as u16;
            if y >= area.y + area.height - 3 {
                break;
            }

            let selected = idx == self.menu_index;
            let prefix = if selected { "▶ " } else { "  " };

            let name = if *item == Path::new("..") {
                "..".to_string()
            } else if show_full_name {
                item.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                let base = item
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if item.is_dir() {
                    format!("{}/", base)
                } else {
                    base
                }
            };

            let text = format!("{}{}", prefix, name);
            let x = area.x + 4;
            let max_width = (area.width - 8) as usize;

            for (j, ch) in text.chars().take(max_width).enumerate() {
                if let Some(cell) = buf.cell_mut((x + j as u16, y)) {
                    cell.set_char(ch);
                    let color = if selected {
                        ctx.theme.accent_color
                    } else if item.is_dir() || *item == Path::new("..") {
                        ctx.theme.text_color
                    } else {
                        ctx.theme.success_color
                    };
                    cell.set_fg(color);
                }
            }
        }

        // Scroll indicator down
        let remaining = items.len().saturating_sub(visible_end);
        if remaining > 0 {
            let indicator = format!("▼ {} more", remaining);
            let y = area.y + area.height - 3;
            let x = area.x + 4;
            for (i, ch) in indicator.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x + i as u16, y)) {
                    cell.set_char(ch);
                    cell.set_fg(ctx.theme.dim_color);
                }
            }
        }
    }

    /// Render pause menu overlay
    fn render_pause_menu(&self, area: Rect, buf: &mut Buffer, ctx: &PluginContext) {
        // Draw title
        let title = "PAUSED";
        let title_y = area.y + 2;
        let title_x = area.x + (area.width.saturating_sub(title.len() as u16)) / 2;
        for (i, ch) in title.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((title_x + i as u16, title_y)) {
                cell.set_char(ch);
                cell.set_fg(ctx.theme.accent_color);
            }
        }

        // Draw game name
        let game_y = title_y + 1;
        let game_name = self
            .rom_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let game_x = area.x + (area.width.saturating_sub(game_name.len() as u16)) / 2;
        for (i, ch) in game_name.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((game_x + i as u16, game_y)) {
                cell.set_char(ch);
                cell.set_fg(ctx.theme.dim_color);
            }
        }

        // Draw menu items
        let items = ["Resume", "Save State", "Load State", "Reset", "Exit ROM"];
        let menu_y = game_y + 3;

        for (i, item) in items.iter().enumerate() {
            let y = menu_y + i as u16;
            if y >= area.y + area.height - 2 {
                break;
            }

            let selected = i == self.pause_index;
            let prefix = if selected { "▶ " } else { "  " };
            let text = format!("{}{}", prefix, item);
            let x = area.x + (area.width.saturating_sub(text.len() as u16)) / 2;

            for (j, ch) in text.chars().enumerate() {
                if let Some(cell) = buf.cell_mut((x + j as u16, y)) {
                    cell.set_char(ch);
                    cell.set_fg(if selected {
                        ctx.theme.accent_color
                    } else {
                        ctx.theme.text_color
                    });
                }
            }
        }

        // Draw footer
        let footer_y = area.y + area.height - 2;
        let footer = "↑↓: Navigate  Enter: Select  Esc: Resume";
        let footer_x = area.x + (area.width.saturating_sub(footer.len() as u16)) / 2;
        for (i, ch) in footer.chars().enumerate() {
            if let Some(cell) = buf.cell_mut((footer_x + i as u16, footer_y)) {
                cell.set_char(ch);
                cell.set_fg(ctx.theme.dim_color);
            }
        }
    }
}

impl Default for RetroArchPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for RetroArchPlugin {
    fn id(&self) -> &str {
        "retroarch"
    }

    fn name(&self) -> &str {
        "RetroArch"
    }

    fn display_name(&self) -> String {
        match &self.plugin_state {
            RetroArchState::Playing => format!("RetroArch: {}", self.core_name),
            RetroArchState::Paused => format!("RetroArch: {} (Paused)", self.core_name),
            RetroArchState::Menu(_) => {
                if self.error.is_some() {
                    "RetroArch (Error)".to_string()
                } else {
                    "RetroArch".to_string()
                }
            }
        }
    }

    fn render_mode(&self) -> PluginRenderMode {
        match &self.plugin_state {
            RetroArchState::Playing => PluginRenderMode::KittyGraphics,
            _ => PluginRenderMode::Text,
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &PluginContext) {
        // Text mode rendering for menus and paused state
        match &self.plugin_state {
            RetroArchState::Menu(menu_state) => {
                self.render_menu(area, buf, ctx, menu_state);
            }
            RetroArchState::Paused => {
                self.render_pause_menu(area, buf, ctx);
            }
            RetroArchState::Playing => {
                // KittyGraphics mode - render_frame() is used instead
            }
        }
    }

    fn render_frame(&mut self, _width: u32, _height: u32) -> Option<PluginFrame> {
        let state = self.shared_state.lock();
        if !state.frame_ready || state.frame_buffer.is_empty() {
            tracing::trace!(
                "render_frame: no frame (ready={}, buf_len={})",
                state.frame_ready,
                state.frame_buffer.len()
            );
            return None;
        }

        tracing::trace!(
            "render_frame: returning frame {}x{} ({} bytes)",
            state.frame_width,
            state.frame_height,
            state.frame_buffer.len()
        );

        // Zero-copy: use Arc::clone() instead of Vec::clone()
        Some(PluginFrame::from_arc(
            Arc::clone(&state.frame_buffer),
            state.frame_width,
            state.frame_height,
        ))
    }

    fn handle_event(&mut self, event: &Event, _area: Rect) -> PluginEventResult {
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event
        {
            match &self.plugin_state {
                RetroArchState::Playing => {
                    // In Playing state, check for pause first
                    if *code == KeyCode::Esc {
                        self.plugin_state = RetroArchState::Paused;
                        self.pause_index = 0;
                        return PluginEventResult::Consumed;
                    }

                    // Pass game controls to libretro
                    if let Some(button) = Self::key_to_button(*code, *modifiers) {
                        let mut state = self.shared_state.lock();
                        let button_id = button as usize;
                        if button_id < 16 {
                            state.button_press_frame[button_id] = state.frame_count;
                        }
                        return PluginEventResult::Consumed;
                    }
                }
                RetroArchState::Menu(_) | RetroArchState::Paused => {
                    // Handle menu navigation
                    match code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            if matches!(self.plugin_state, RetroArchState::Paused) {
                                self.pause_index = self.pause_index.saturating_sub(1);
                            } else {
                                self.menu_navigate(-1);
                            }
                            return PluginEventResult::Consumed;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if matches!(self.plugin_state, RetroArchState::Paused) {
                                self.pause_index = (self.pause_index + 1).min(4);
                            } else {
                                self.menu_navigate(1);
                            }
                            return PluginEventResult::Consumed;
                        }
                        KeyCode::Enter => {
                            self.menu_select();
                            return PluginEventResult::Consumed;
                        }
                        KeyCode::Esc | KeyCode::Backspace => {
                            if self.menu_back() {
                                return PluginEventResult::Consumed;
                            }
                            // Let parent handle if we can't go back
                            return PluginEventResult::Ignored;
                        }
                        _ => {}
                    }
                }
            }
        }
        PluginEventResult::Ignored
    }

    fn tick(&mut self) -> bool {
        // Only run frames when in Playing state
        if self.plugin_state == RetroArchState::Playing && self.running {
            // Clear frame ready flag and increment frame counter
            {
                let mut state = self.shared_state.lock();
                state.frame_ready = false;
                state.frame_count = state.frame_count.wrapping_add(1);
            }

            // Run one frame
            self.run_frame();

            // Check if frame was produced
            let state = self.shared_state.lock();
            // Log every 300 frames (~5 seconds) to avoid spam
            if state.frame_count.is_multiple_of(300) {
                tracing::info!(
                    "RetroArch: frame_count={}, ready={}, buf={}B, dims={}x{}",
                    state.frame_count,
                    state.frame_ready,
                    state.frame_buffer.len(),
                    state.frame_width,
                    state.frame_height
                );
            }
            state.frame_ready
        } else {
            // Menu/Paused states need periodic redraws for cursor blink etc
            matches!(
                self.plugin_state,
                RetroArchState::Menu(_) | RetroArchState::Paused
            )
        }
    }

    fn on_activate(&mut self) {
        set_shared_state(self.shared_state.clone());

        // Start in menu state - no auto-start
        // Users navigate: Load Game -> Select Core -> Select ROM
        if self.core.is_none() {
            // Auto-load from environment variables ONLY if both are set
            // This allows users who set env vars to still auto-start
            if let (Ok(core), Ok(rom)) = (
                std::env::var("KRUSTY_RETROARCH_CORE"),
                std::env::var("KRUSTY_RETROARCH_ROM"),
            ) {
                let core_path = PathBuf::from(core);
                let rom_path = PathBuf::from(rom);

                if core_path.exists() && rom_path.exists() {
                    tracing::info!("RetroArch: Auto-loading from env vars");
                    if let Err(e) = self.load_core(&core_path) {
                        self.error = Some(format!("Core load failed: {}", e));
                    } else if let Err(e) = self.load_rom(&rom_path) {
                        self.error = Some(format!("ROM load failed: {}", e));
                    } else {
                        self.plugin_state = RetroArchState::Playing;
                    }
                }
            }
            // Otherwise stay in menu state (default)
        }
    }

    fn on_deactivate(&mut self) {
        // Don't clear state - keep game running in background
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for RetroArchPlugin {
    fn drop(&mut self) {
        self.unload();
    }
}
