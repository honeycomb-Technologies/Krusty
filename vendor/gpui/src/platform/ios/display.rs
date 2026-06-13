//! iOS display support backed by UIScreen.

use super::cg_types::ObjcCGRect;
use crate::{Bounds, DisplayId, Pixels, PlatformDisplay, px, size};
use anyhow::Result;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct IosDisplay {
    screen: *mut AnyObject,
}

unsafe impl Send for IosDisplay {}
unsafe impl Sync for IosDisplay {}

impl IosDisplay {
    pub(crate) fn main() -> Self {
        unsafe {
            let screen: *mut AnyObject = msg_send![class!(UIScreen), mainScreen];
            Self { screen }
        }
    }

    pub(crate) fn all() -> Vec<Self> {
        unsafe {
            let screens: *mut AnyObject = msg_send![class!(UIScreen), screens];
            let count: usize = msg_send![screens, count];
            (0..count)
                .map(|ix| {
                    let screen: *mut AnyObject = msg_send![screens, objectAtIndex: ix];
                    Self { screen }
                })
                .collect()
        }
    }

    pub(crate) fn scale(&self) -> f32 {
        unsafe {
            let scale: f64 = msg_send![self.screen, scale];
            scale as f32
        }
    }

    fn native_scale(&self) -> f32 {
        unsafe {
            let scale: f64 = msg_send![self.screen, nativeScale];
            scale as f32
        }
    }

    fn bounds_in_points(&self) -> ObjcCGRect {
        unsafe { msg_send![self.screen, bounds] }
    }
}

impl PlatformDisplay for IosDisplay {
    fn id(&self) -> DisplayId {
        DisplayId(self.screen as usize as u32)
    }

    fn uuid(&self) -> Result<Uuid> {
        let bounds = self.bounds_in_points();
        let key = format!(
            "ios-screen-{}-{}-{}",
            bounds.width as u32,
            bounds.height as u32,
            (self.native_scale() * 100.0) as u32
        );
        Ok(Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()))
    }

    fn bounds(&self) -> Bounds<Pixels> {
        let bounds = self.bounds_in_points();
        Bounds {
            origin: Default::default(),
            size: size(px(bounds.width as f32), px(bounds.height as f32)),
        }
    }
}
