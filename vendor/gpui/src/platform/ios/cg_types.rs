//! objc2-encodable CoreGraphics geometry mirrors used by the iOS backend.

use objc2::encode::{Encode, Encoding, RefEncode};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ObjcCGRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

unsafe impl Encode for ObjcCGRect {
    const ENCODING: Encoding = Encoding::Struct(
        "CGRect",
        &[
            Encoding::Struct("CGPoint", &[Encoding::Double, Encoding::Double]),
            Encoding::Struct("CGSize", &[Encoding::Double, Encoding::Double]),
        ],
    );
}

unsafe impl RefEncode for ObjcCGRect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

impl ObjcCGRect {
    pub(crate) fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ObjcCGPoint {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

unsafe impl Encode for ObjcCGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[Encoding::Double, Encoding::Double]);
}

unsafe impl RefEncode for ObjcCGPoint {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ObjcUIEdgeInsets {
    pub(crate) top: f64,
    pub(crate) left: f64,
    pub(crate) bottom: f64,
    pub(crate) right: f64,
}

unsafe impl Encode for ObjcUIEdgeInsets {
    const ENCODING: Encoding = Encoding::Struct(
        "UIEdgeInsets",
        &[
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
        ],
    );
}

unsafe impl RefEncode for ObjcUIEdgeInsets {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}
