//! LibRetro FFI Bindings
//!
//! Low-level bindings to the libretro API for loading and running emulator cores.
//! Reference: https://docs.libretro.com/
//!
//! Note: This module defines the complete libretro API. Some variants and fields
//! are not yet used but are included for API completeness and future expansion.

// FFI bindings define the complete API even if not all parts are used yet
#![allow(dead_code)]

use std::ffi::{c_char, c_void, CStr};
use std::path::Path;

/// LibRetro pixel format
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 0RGB1555, native endian (5 bits per RGB channel)
    RGB1555 = 0,
    /// XRGB8888, native endian (8 bits per RGB channel)
    XRGB8888 = 1,
    /// RGB565, native endian (5-6-5 bits)
    RGB565 = 2,
}

/// LibRetro device types for input
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    None = 0,
    Joypad = 1,
    Mouse = 2,
    Keyboard = 3,
    LightGun = 4,
    Analog = 5,
    Pointer = 6,
}

/// Joypad button IDs
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoypadButton {
    B = 0,
    Y = 1,
    Select = 2,
    Start = 3,
    Up = 4,
    Down = 5,
    Left = 6,
    Right = 7,
    A = 8,
    X = 9,
    L = 10,
    R = 11,
    L2 = 12,
    R2 = 13,
    L3 = 14,
    R3 = 15,
}

/// System AV info returned by the core
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SystemAvInfo {
    pub geometry: GameGeometry,
    pub timing: SystemTiming,
}

/// Game geometry (resolution info)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GameGeometry {
    pub base_width: u32,
    pub base_height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub aspect_ratio: f32,
}

/// System timing (fps and sample rate)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

/// Game info for loading ROMs
#[repr(C)]
pub struct GameInfo {
    pub path: *const c_char,
    pub data: *const c_void,
    pub size: usize,
    pub meta: *const c_char,
}

/// System info about the core
#[repr(C)]
pub struct SystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

/// Environment command IDs
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentCmd {
    SetRotation = 1,
    GetOverscan = 2,
    GetCanDupe = 3,
    SetMessage = 6,
    Shutdown = 7,
    SetPerformanceLevel = 8,
    GetSystemDirectory = 9,
    SetPixelFormat = 10,
    SetInputDescriptors = 11,
    SetKeyboardCallback = 12,
    SetDiskControlInterface = 13,
    SetHwRender = 14,
    GetVariable = 15,
    SetVariables = 16,
    GetVariableUpdate = 17,
    SetSupportNoGame = 18,
    GetLibretroPath = 19,
    SetFrameTimeCallback = 21,
    SetAudioCallback = 22,
    GetRumbleInterface = 23,
    GetInputDeviceCapabilities = 24,
    GetSensorInterface = 25,
    GetCameraInterface = 26,
    GetLogInterface = 27,
    GetPerfInterface = 28,
    GetLocationInterface = 29,
    GetCoreAssetsDirectory = 30,
    GetSaveDirectory = 31,
    SetSystemAvInfo = 32,
    SetProcAddressCallback = 33,
    SetSubsystemInfo = 34,
    SetControllerInfo = 35,
    SetMemoryMaps = 36,
    SetGeometry = 37,
    GetUsername = 38,
    GetLanguage = 39,
}

/// Callback types used by libretro
pub type VideoRefreshFn = extern "C" fn(data: *const c_void, width: u32, height: u32, pitch: usize);
pub type AudioSampleFn = extern "C" fn(left: i16, right: i16);
pub type AudioSampleBatchFn = extern "C" fn(data: *const i16, frames: usize) -> usize;
pub type InputPollFn = extern "C" fn();
pub type InputStateFn = extern "C" fn(port: u32, device: u32, index: u32, id: u32) -> i16;
pub type EnvironmentFn = extern "C" fn(cmd: u32, data: *mut c_void) -> bool;

/// Memory type IDs for retro_get_memory_*
pub const RETRO_MEMORY_SAVE_RAM: u32 = 0;
pub const RETRO_MEMORY_RTC: u32 = 1;
pub const RETRO_MEMORY_SYSTEM_RAM: u32 = 2;
pub const RETRO_MEMORY_VIDEO_RAM: u32 = 3;

/// Loaded libretro core with function pointers
pub struct LibRetroCore {
    // Dynamic library handle
    #[cfg(unix)]
    _lib: libloading::Library,

    // Core functions
    pub retro_init: extern "C" fn(),
    pub retro_deinit: extern "C" fn(),
    pub retro_api_version: extern "C" fn() -> u32,
    pub retro_get_system_info: extern "C" fn(info: *mut SystemInfo),
    pub retro_get_system_av_info: extern "C" fn(info: *mut SystemAvInfo),
    pub retro_set_controller_port_device: extern "C" fn(port: u32, device: u32),
    pub retro_reset: extern "C" fn(),
    pub retro_run: extern "C" fn(),
    pub retro_serialize_size: extern "C" fn() -> usize,
    pub retro_serialize: extern "C" fn(data: *mut c_void, size: usize) -> bool,
    pub retro_unserialize: extern "C" fn(data: *const c_void, size: usize) -> bool,
    pub retro_load_game: extern "C" fn(game: *const GameInfo) -> bool,
    pub retro_unload_game: extern "C" fn(),
    pub retro_get_region: extern "C" fn() -> u32,
    pub retro_get_memory_data: extern "C" fn(id: u32) -> *mut c_void,
    pub retro_get_memory_size: extern "C" fn(id: u32) -> usize,

    // Callback setters
    pub retro_set_environment: extern "C" fn(EnvironmentFn),
    pub retro_set_video_refresh: extern "C" fn(VideoRefreshFn),
    pub retro_set_audio_sample: extern "C" fn(AudioSampleFn),
    pub retro_set_audio_sample_batch: extern "C" fn(AudioSampleBatchFn),
    pub retro_set_input_poll: extern "C" fn(InputPollFn),
    pub retro_set_input_state: extern "C" fn(InputStateFn),
}

impl LibRetroCore {
    /// Load a libretro core from a dynamic library
    ///
    /// # Safety
    /// The core must be a valid libretro core implementing the required API.
    pub unsafe fn load(path: &Path) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use libloading::{Library, Symbol};

            let lib = Library::new(path).map_err(|e| format!("Failed to load core: {}", e))?;

            macro_rules! get_fn {
                ($name:ident, $type:ty) => {
                    **lib
                        .get::<Symbol<$type>>(stringify!($name).as_bytes())
                        .map_err(|e| format!("Missing symbol {}: {}", stringify!($name), e))?
                };
            }

            let core = LibRetroCore {
                retro_init: get_fn!(retro_init, extern "C" fn()),
                retro_deinit: get_fn!(retro_deinit, extern "C" fn()),
                retro_api_version: get_fn!(retro_api_version, extern "C" fn() -> u32),
                retro_get_system_info: get_fn!(
                    retro_get_system_info,
                    extern "C" fn(*mut SystemInfo)
                ),
                retro_get_system_av_info: get_fn!(
                    retro_get_system_av_info,
                    extern "C" fn(*mut SystemAvInfo)
                ),
                retro_set_controller_port_device: get_fn!(
                    retro_set_controller_port_device,
                    extern "C" fn(u32, u32)
                ),
                retro_reset: get_fn!(retro_reset, extern "C" fn()),
                retro_run: get_fn!(retro_run, extern "C" fn()),
                retro_serialize_size: get_fn!(retro_serialize_size, extern "C" fn() -> usize),
                retro_serialize: get_fn!(
                    retro_serialize,
                    extern "C" fn(*mut c_void, usize) -> bool
                ),
                retro_unserialize: get_fn!(
                    retro_unserialize,
                    extern "C" fn(*const c_void, usize) -> bool
                ),
                retro_load_game: get_fn!(retro_load_game, extern "C" fn(*const GameInfo) -> bool),
                retro_unload_game: get_fn!(retro_unload_game, extern "C" fn()),
                retro_get_region: get_fn!(retro_get_region, extern "C" fn() -> u32),
                retro_get_memory_data: get_fn!(
                    retro_get_memory_data,
                    extern "C" fn(u32) -> *mut c_void
                ),
                retro_get_memory_size: get_fn!(retro_get_memory_size, extern "C" fn(u32) -> usize),
                retro_set_environment: get_fn!(retro_set_environment, extern "C" fn(EnvironmentFn)),
                retro_set_video_refresh: get_fn!(
                    retro_set_video_refresh,
                    extern "C" fn(VideoRefreshFn)
                ),
                retro_set_audio_sample: get_fn!(
                    retro_set_audio_sample,
                    extern "C" fn(AudioSampleFn)
                ),
                retro_set_audio_sample_batch: get_fn!(
                    retro_set_audio_sample_batch,
                    extern "C" fn(AudioSampleBatchFn)
                ),
                retro_set_input_poll: get_fn!(retro_set_input_poll, extern "C" fn(InputPollFn)),
                retro_set_input_state: get_fn!(retro_set_input_state, extern "C" fn(InputStateFn)),
                _lib: lib,
            };

            // Verify API version
            let version = (core.retro_api_version)();
            if version != 1 {
                return Err(format!("Unsupported libretro API version: {}", version));
            }

            Ok(core)
        }

        #[cfg(not(unix))]
        {
            Err("LibRetro loading not yet supported on this platform".to_string())
        }
    }

    /// Get system info from the core
    pub fn get_system_info(&self) -> SystemInfo {
        let mut info = SystemInfo {
            library_name: std::ptr::null(),
            library_version: std::ptr::null(),
            valid_extensions: std::ptr::null(),
            need_fullpath: false,
            block_extract: false,
        };
        (self.retro_get_system_info)(&mut info);
        info
    }

    /// Get the core's name as a string
    pub fn name(&self) -> String {
        let info = self.get_system_info();
        if info.library_name.is_null() {
            return "Unknown".to_string();
        }
        unsafe { CStr::from_ptr(info.library_name) }
            .to_string_lossy()
            .to_string()
    }

    /// Get the core's version as a string
    pub fn version(&self) -> String {
        let info = self.get_system_info();
        if info.library_version.is_null() {
            return "Unknown".to_string();
        }
        unsafe { CStr::from_ptr(info.library_version) }
            .to_string_lossy()
            .to_string()
    }
}
