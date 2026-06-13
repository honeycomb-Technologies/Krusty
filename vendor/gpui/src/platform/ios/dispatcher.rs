//! iOS dispatcher backed by Grand Central Dispatch.

use crate::{PlatformDispatcher, TaskLabel};
use async_task::Runnable;
use objc2::runtime::Bool;
use objc2::{class, msg_send};
use std::{ffi::c_void, ptr::NonNull, time::Duration};

type DispatchQueue = *mut c_void;
type DispatchTime = u64;

const DISPATCH_TIME_NOW: DispatchTime = 0;
const DISPATCH_QUEUE_PRIORITY_DEFAULT: i64 = 0;

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: DispatchQueue,
        context: *mut c_void,
        work: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    fn dispatch_after_f(
        when: DispatchTime,
        queue: DispatchQueue,
        context: *mut c_void,
        work: Option<unsafe extern "C" fn(*mut c_void)>,
    );
    fn dispatch_get_global_queue(identifier: i64, flags: u64) -> DispatchQueue;
    fn dispatch_time(when: DispatchTime, delta: i64) -> DispatchTime;
}

fn main_queue() -> DispatchQueue {
    std::ptr::addr_of!(_dispatch_main_q) as *const _ as DispatchQueue
}

pub(crate) struct IosDispatcher;

impl PlatformDispatcher for IosDispatcher {
    fn is_main_thread(&self) -> bool {
        unsafe {
            let is_main: Bool = msg_send![class!(NSThread), isMainThread];
            is_main.as_bool()
        }
    }

    fn dispatch(&self, runnable: Runnable, _label: Option<TaskLabel>) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;
        unsafe {
            dispatch_async_f(
                dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0),
                context,
                Some(trampoline),
            );
        }
    }

    fn dispatch_on_main_thread(&self, runnable: Runnable) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;
        unsafe {
            dispatch_async_f(main_queue(), context, Some(trampoline));
        }
    }

    fn dispatch_after(&self, duration: Duration, runnable: Runnable) {
        let context = runnable.into_raw().as_ptr() as *mut c_void;
        unsafe {
            dispatch_after_f(
                dispatch_time(DISPATCH_TIME_NOW, duration.as_nanos() as i64),
                dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0),
                context,
                Some(trampoline),
            );
        }
    }
}

unsafe extern "C" fn trampoline(runnable: *mut c_void) {
    let runnable = unsafe { Runnable::from_raw(NonNull::new_unchecked(runnable as *mut ())) };
    runnable.run();
}
