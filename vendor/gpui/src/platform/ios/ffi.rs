//! C ABI hooks for a Swift/Objective-C shell to drive the iOS GPUI backend.

use super::IosWindow;
use std::{cell::UnsafeCell, ffi::c_void, sync::OnceLock};

pub(crate) struct WindowList(UnsafeCell<Vec<*const IosWindow>>);

unsafe impl Send for WindowList {}
unsafe impl Sync for WindowList {}

static WINDOWS: OnceLock<WindowList> = OnceLock::new();
static HOST_VIEW: OnceLock<HostViewCell> = OnceLock::new();

pub(crate) struct HostViewCell(UnsafeCell<*mut c_void>);

unsafe impl Send for HostViewCell {}
unsafe impl Sync for HostViewCell {}

fn windows() -> &'static WindowList {
    WINDOWS.get_or_init(|| WindowList(UnsafeCell::new(Vec::new())))
}

fn host_view() -> &'static HostViewCell {
    HOST_VIEW.get_or_init(|| HostViewCell(UnsafeCell::new(std::ptr::null_mut())))
}

pub(crate) fn configured_host_view() -> *mut c_void {
    unsafe { *host_view().0.get() }
}

pub(crate) fn register_window(window: *const IosWindow) {
    unsafe {
        (*windows().0.get()).push(window);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_set_host_view(view: *mut c_void) {
    unsafe {
        *host_view().0.get() = view;
    }
}

pub(crate) fn for_each_window(mut callback: impl FnMut(&IosWindow)) {
    unsafe {
        for &window in (*windows().0.get()).iter() {
            if let Some(window) = window.as_ref() {
                callback(window);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_get_window() -> *mut c_void {
    unsafe {
        (*windows().0.get())
            .last()
            .copied()
            .unwrap_or(std::ptr::null()) as *mut c_void
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_request_frame(window: *mut c_void) {
    if let Some(window) = unsafe { (window as *const IosWindow).as_ref() } {
        window.request_frame(false);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_force_frame(window: *mut c_void) {
    if let Some(window) = unsafe { (window as *const IosWindow).as_ref() } {
        window.request_frame(true);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_layout_windows() {
    for_each_window(|window| window.handle_layout_change());
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_will_enter_foreground() {
    for_each_window(|window| window.set_active(true));
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_did_enter_background() {
    for_each_window(|window| window.set_active(false));
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_show_keyboard(window: *mut c_void) {
    if let Some(window) = unsafe { (window as *const IosWindow).as_ref() } {
        window.show_keyboard();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_hide_keyboard(window: *mut c_void) {
    if let Some(window) = unsafe { (window as *const IosWindow).as_ref() } {
        window.hide_keyboard();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gpui_ios_safe_area_insets(
    window: *mut c_void,
    top: *mut f32,
    bottom: *mut f32,
    left: *mut f32,
    right: *mut f32,
) -> bool {
    let Some(window) = (unsafe { (window as *const IosWindow).as_ref() }) else {
        return false;
    };
    let (t, b, l, r) = window.safe_area_insets();
    unsafe {
        if !top.is_null() {
            *top = t;
        }
        if !bottom.is_null() {
            *bottom = b;
        }
        if !left.is_null() {
            *left = l;
        }
        if !right.is_null() {
            *right = r;
        }
    }
    true
}
