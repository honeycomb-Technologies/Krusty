//! iOS platform backend for Krusty's vendored GPUI 0.2.2.

mod cg_types;
mod dispatcher;
mod display;
pub(crate) mod ffi;
mod platform;
#[cfg(feature = "font-kit")]
mod text_system;
mod util;
mod window;

pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use platform::*;
#[cfg(feature = "font-kit")]
pub(crate) use text_system::*;
pub(crate) use window::*;

pub(crate) type PlatformScreenCaptureFrame = ();

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {}
