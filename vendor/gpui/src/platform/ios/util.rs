//! Shared iOS Objective-C helpers.

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};

/// Create an autoreleased NSString from a Rust string.
///
/// # Safety
/// The caller must be running on an Objective-C/UIKit thread with an autorelease pool.
pub(crate) unsafe fn nsstring(s: &str) -> *mut AnyObject {
    let ns: *mut AnyObject = unsafe { msg_send![class!(NSString), alloc] };
    let ns: *mut AnyObject = unsafe {
        msg_send![ns,
            initWithBytes: s.as_ptr() as *const std::ffi::c_void,
            length: s.len(),
            encoding: 4u64 // NSUTF8StringEncoding
        ]
    };
    unsafe { msg_send![ns, autorelease] }
}

pub(crate) unsafe fn nsstring_to_string(value: *mut AnyObject) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let utf8: *const std::ffi::c_char = unsafe { msg_send![value, UTF8String] };
    if utf8.is_null() {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(utf8) }
            .to_string_lossy()
            .into_owned(),
    )
}
