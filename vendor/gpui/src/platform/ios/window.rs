//! iOS window support backed by UIWindow, UIViewController, CAMetalLayer and Blade.

use super::{IosDisplay, cg_types::*};
use crate::platform::blade::{BladeContext, BladeRenderer, BladeSurfaceConfig};
use crate::{
    AnyWindowHandle, Bounds, Capslock, DevicePixels, DispatchEventResult, GpuSpecs, Keystroke,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, PlatformAtlas,
    PlatformDisplay, PlatformInput, PlatformInputHandler, PlatformWindow, Point, PromptButton,
    PromptLevel, RequestFrameOptions, Scene, ScrollDelta, ScrollWheelEvent, Size, TouchPhase,
    WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowParams,
    point, px, size,
};
use blade_graphics as gpu;
use futures::channel::oneshot;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use parking_lot::Mutex;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, UiKitDisplayHandle, UiKitWindowHandle};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    ptr::{self, NonNull},
    rc::Rc,
    sync::{Arc, Once},
};

const GPUI_WINDOW_IVAR: &str = "gpui_window_ptr";
const SCROLL_SLOP: f32 = 8.0;

static METAL_VIEW_CLASS_REGISTERED: Once = Once::new();
static VIEW_CONTROLLER_CLASS_REGISTERED: Once = Once::new();
static TEXT_INPUT_VIEW_CLASS_REGISTERED: Once = Once::new();

#[derive(Clone, Copy, Debug)]
struct RawIosWindow {
    view: *mut c_void,
}

unsafe impl Send for RawIosWindow {}
unsafe impl Sync for RawIosWindow {}

impl HasWindowHandle for RawIosWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        let view = NonNull::new(self.view).ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = UiKitWindowHandle::new(view);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for RawIosWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError>
    {
        let handle = UiKitDisplayHandle::new();
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(handle.into()) })
    }
}

#[derive(Clone, Copy, Debug)]
enum TouchState {
    Idle,
    Pending { start_x: f32, start_y: f32 },
    Scrolling { prev_x: f32, prev_y: f32 },
}

fn register_view_controller_class() -> &'static AnyClass {
    VIEW_CONTROLLER_CLASS_REGISTERED.call_once(|| {
        let superclass = class!(UIViewController);
        let mut decl = ClassBuilder::new(c"GPUIIOSViewController", superclass).unwrap();

        extern "C" fn view_did_layout_subviews(this: *mut AnyObject, _sel: Sel) {
            unsafe {
                let superclass = class!(UIViewController);
                let _: () = msg_send![super(this, superclass), viewDidLayoutSubviews];
            }
            super::ffi::for_each_window(|window| window.handle_layout_change());
        }

        unsafe {
            decl.add_method(
                sel!(viewDidLayoutSubviews),
                view_did_layout_subviews as extern "C" fn(*mut AnyObject, Sel),
            );
        }
        decl.register();
    });
    class!(GPUIIOSViewController)
}

fn register_metal_view_class() -> &'static AnyClass {
    METAL_VIEW_CLASS_REGISTERED.call_once(|| {
        let superclass = class!(UIView);
        let mut decl = ClassBuilder::new(c"GPUIIOSMetalView", superclass).unwrap();
        decl.add_ivar::<*mut c_void>(c"gpui_window_ptr");

        extern "C" fn layer_class(_this: *const AnyClass, _sel: Sel) -> *const AnyClass {
            class!(CAMetalLayer) as *const AnyClass
        }
        extern "C" fn touches_began(
            this: *mut AnyObject,
            _sel: Sel,
            touches: *mut AnyObject,
            event: *mut AnyObject,
        ) {
            handle_touches(this, touches, event);
        }
        extern "C" fn touches_moved(
            this: *mut AnyObject,
            _sel: Sel,
            touches: *mut AnyObject,
            event: *mut AnyObject,
        ) {
            handle_touches(this, touches, event);
        }
        extern "C" fn touches_ended(
            this: *mut AnyObject,
            _sel: Sel,
            touches: *mut AnyObject,
            event: *mut AnyObject,
        ) {
            handle_touches(this, touches, event);
        }
        extern "C" fn touches_cancelled(
            this: *mut AnyObject,
            _sel: Sel,
            touches: *mut AnyObject,
            event: *mut AnyObject,
        ) {
            handle_touches(this, touches, event);
        }

        unsafe {
            decl.add_class_method(
                sel!(layerClass),
                layer_class as extern "C" fn(*const AnyClass, Sel) -> *const AnyClass,
            );
            decl.add_method(
                sel!(touchesBegan:withEvent:),
                touches_began as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
            decl.add_method(
                sel!(touchesMoved:withEvent:),
                touches_moved as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
            decl.add_method(
                sel!(touchesEnded:withEvent:),
                touches_ended as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
            decl.add_method(
                sel!(touchesCancelled:withEvent:),
                touches_cancelled
                    as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
            );
        }
        decl.register();
    });
    class!(GPUIIOSMetalView)
}

fn register_text_input_view_class() -> &'static AnyClass {
    TEXT_INPUT_VIEW_CLASS_REGISTERED.call_once(|| {
        let superclass = class!(UIView);
        let mut decl = ClassBuilder::new(c"GPUIIOSTextInputView", superclass).unwrap();
        if let Some(protocol) = AnyProtocol::get(c"UIKeyInput") {
            decl.add_protocol(protocol);
        }
        decl.add_ivar::<*mut c_void>(c"gpui_window_ptr");
        decl.add_ivar::<isize>(c"_keyboardType");

        extern "C" fn has_text(_this: *mut AnyObject, _sel: Sel) -> Bool {
            Bool::YES
        }
        extern "C" fn can_become_first_responder(_this: *mut AnyObject, _sel: Sel) -> Bool {
            Bool::YES
        }
        extern "C" fn insert_text(this: *mut AnyObject, _sel: Sel, text: *mut AnyObject) {
            let window_ptr: *mut c_void = unsafe {
                #[allow(deprecated)]
                *(*this).get_ivar(GPUI_WINDOW_IVAR)
            };
            if let Some(window) = unsafe { (window_ptr as *const IosWindow).as_ref() } {
                window.handle_text_input(text);
            }
        }
        extern "C" fn delete_backward(this: *mut AnyObject, _sel: Sel) {
            let window_ptr: *mut c_void = unsafe {
                #[allow(deprecated)]
                *(*this).get_ivar(GPUI_WINDOW_IVAR)
            };
            if let Some(window) = unsafe { (window_ptr as *const IosWindow).as_ref() } {
                window.handle_delete_backward();
            }
        }
        extern "C" fn get_keyboard_type(this: *mut AnyObject, _sel: Sel) -> isize {
            unsafe {
                #[allow(deprecated)]
                *(*this).get_ivar::<isize>("_keyboardType")
            }
        }
        extern "C" fn set_keyboard_type(this: *mut AnyObject, _sel: Sel, val: isize) {
            unsafe {
                #[allow(deprecated)]
                *(*this).get_mut_ivar::<isize>("_keyboardType") = val;
            }
        }

        unsafe {
            decl.add_method(
                sel!(hasText),
                has_text as extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
            decl.add_method(
                sel!(canBecomeFirstResponder),
                can_become_first_responder as extern "C" fn(*mut AnyObject, Sel) -> Bool,
            );
            decl.add_method(
                sel!(insertText:),
                insert_text as extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            decl.add_method(
                sel!(deleteBackward),
                delete_backward as extern "C" fn(*mut AnyObject, Sel),
            );
            decl.add_method(
                sel!(keyboardType),
                get_keyboard_type as extern "C" fn(*mut AnyObject, Sel) -> isize,
            );
            decl.add_method(
                sel!(setKeyboardType:),
                set_keyboard_type as extern "C" fn(*mut AnyObject, Sel, isize),
            );
        }
        decl.register();
    });
    class!(GPUIIOSTextInputView)
}

fn handle_touches(view: *mut AnyObject, touches: *mut AnyObject, _event: *mut AnyObject) {
    unsafe {
        #[allow(deprecated)]
        let window_ptr: *mut c_void = *(*view).get_ivar(GPUI_WINDOW_IVAR);
        let Some(window) = (window_ptr as *const IosWindow).as_ref() else {
            return;
        };
        let all_touches: *mut AnyObject = msg_send![touches, allObjects];
        let count: usize = msg_send![all_touches, count];
        for ix in 0..count {
            let touch: *mut AnyObject = msg_send![all_touches, objectAtIndex: ix];
            window.handle_touch(touch);
        }
    }
}

pub(crate) struct IosWindow {
    window: *mut AnyObject,
    view_controller: *mut AnyObject,
    view: *mut AnyObject,
    text_input_view: *mut AnyObject,
    bounds: Cell<Bounds<Pixels>>,
    scale_factor: Cell<f32>,
    input_handler: RefCell<Option<PlatformInputHandler>>,
    request_frame_callback: RefCell<Option<Box<dyn FnMut(RequestFrameOptions)>>>,
    input_callback: RefCell<Option<Box<dyn FnMut(PlatformInput) -> DispatchEventResult>>>,
    active_status_callback: RefCell<Option<Box<dyn FnMut(bool)>>>,
    hover_status_callback: RefCell<Option<Box<dyn FnMut(bool)>>>,
    resize_callback: RefCell<Option<Box<dyn FnMut(Size<Pixels>, f32)>>>,
    moved_callback: RefCell<Option<Box<dyn FnMut()>>>,
    should_close_callback: RefCell<Option<Box<dyn FnMut() -> bool>>>,
    hit_test_callback: RefCell<Option<Box<dyn FnMut() -> Option<WindowControlArea>>>>,
    close_callback: RefCell<Option<Box<dyn FnOnce()>>>,
    appearance_changed_callback: RefCell<Option<Box<dyn FnMut()>>>,
    mouse_position: Cell<Point<Pixels>>,
    touch_state: Cell<TouchState>,
    renderer: Mutex<BladeRenderer>,
}

unsafe impl Send for IosWindow {}
unsafe impl Sync for IosWindow {}

impl IosWindow {
    pub(crate) fn new(
        _handle: AnyWindowHandle,
        _params: WindowParams,
        blade_context: Arc<BladeContext>,
    ) -> anyhow::Result<Self> {
        unsafe {
            let screen: *mut AnyObject = msg_send![class!(UIScreen), mainScreen];
            let screen_bounds: ObjcCGRect = msg_send![screen, bounds];
            let scale: f64 = msg_send![screen, scale];
            let host_view = super::ffi::configured_host_view() as *mut AnyObject;

            let view_class = register_metal_view_class();
            let view: *mut AnyObject = msg_send![view_class, alloc];

            let (window, view_controller, view, view_bounds) = if host_view.is_null() {
                let window: *mut AnyObject = msg_send![class!(UIWindow), alloc];
                let window: *mut AnyObject = msg_send![window, initWithFrame: screen_bounds];

                let vc_class = register_view_controller_class();
                let view_controller: *mut AnyObject = msg_send![vc_class, alloc];
                let view_controller: *mut AnyObject = msg_send![view_controller, init];

                let view: *mut AnyObject = msg_send![view, initWithFrame: screen_bounds];
                let _: () = msg_send![view_controller, setView: view];
                let _: () = msg_send![window, setRootViewController: view_controller];
                let _: () = msg_send![window, makeKeyAndVisible];
                (window, view_controller, view, screen_bounds)
            } else {
                let host_bounds: ObjcCGRect = msg_send![host_view, bounds];
                let view: *mut AnyObject = msg_send![view, initWithFrame: host_bounds];
                let _: () = msg_send![host_view, addSubview: view];
                let window: *mut AnyObject = msg_send![host_view, window];
                (window, ptr::null_mut(), view, host_bounds)
            };

            let _: () = msg_send![view, setUserInteractionEnabled: true];
            let _: () = msg_send![view, setMultipleTouchEnabled: true];
            let _: () = msg_send![view, setAutoresizingMask: 18_usize];

            let layer: *mut AnyObject = msg_send![view, layer];
            let _: () = msg_send![layer, setContentsScale: scale];

            let text_input_class = register_text_input_view_class();
            let text_input_view: *mut AnyObject = msg_send![text_input_class, alloc];
            let text_input_view: *mut AnyObject =
                msg_send![text_input_view, initWithFrame: ObjcCGRect::new(0.0, 0.0, 1.0, 1.0)];
            let _: () = msg_send![text_input_view, setAlpha: 0.01_f64];
            let _: () = msg_send![view, addSubview: text_input_view];

            let raw_window = RawIosWindow {
                view: view as *mut c_void,
            };
            let pixel_width = (view_bounds.width * scale).max(1.0) as u32;
            let pixel_height = (view_bounds.height * scale).max(1.0) as u32;
            let renderer = BladeRenderer::new(
                &blade_context,
                &raw_window,
                BladeSurfaceConfig {
                    size: gpu::Extent {
                        width: pixel_width,
                        height: pixel_height,
                        depth: 1,
                    },
                    transparent: false,
                },
            )?;

            Ok(Self {
                window,
                view_controller,
                view,
                text_input_view,
                bounds: Cell::new(Bounds {
                    origin: Default::default(),
                    size: size(px(view_bounds.width as f32), px(view_bounds.height as f32)),
                }),
                scale_factor: Cell::new(scale as f32),
                input_handler: RefCell::new(None),
                request_frame_callback: RefCell::new(None),
                input_callback: RefCell::new(None),
                active_status_callback: RefCell::new(None),
                hover_status_callback: RefCell::new(None),
                resize_callback: RefCell::new(None),
                moved_callback: RefCell::new(None),
                should_close_callback: RefCell::new(None),
                hit_test_callback: RefCell::new(None),
                close_callback: RefCell::new(None),
                appearance_changed_callback: RefCell::new(None),
                mouse_position: Cell::new(Point::default()),
                touch_state: Cell::new(TouchState::Idle),
                renderer: Mutex::new(renderer),
            })
        }
    }

    pub(crate) fn register_with_ffi(&self) {
        super::ffi::register_window(self as *const Self);
        unsafe {
            let window_ptr = self as *const Self as *mut c_void;
            #[allow(deprecated)]
            {
                *(*self.view).get_mut_ivar::<*mut c_void>(GPUI_WINDOW_IVAR) = window_ptr;
            }
            #[allow(deprecated)]
            {
                *(*self.text_input_view).get_mut_ivar::<*mut c_void>(GPUI_WINDOW_IVAR) = window_ptr;
            }
        }
    }

    pub(crate) fn request_frame(&self, force_render: bool) {
        self.handle_layout_change();
        let callback = self.request_frame_callback.borrow_mut().take();
        if let Some(mut callback) = callback {
            callback(RequestFrameOptions {
                force_render,
                ..Default::default()
            });
            if self.request_frame_callback.borrow().is_none() {
                self.request_frame_callback.borrow_mut().replace(callback);
            }
        }
    }

    pub(crate) fn set_active(&self, active: bool) {
        if let Some(callback) = self.active_status_callback.borrow_mut().as_mut() {
            callback(active);
        }
    }

    pub(crate) fn handle_layout_change(&self) {
        unsafe {
            let bounds: ObjcCGRect = msg_send![self.view, bounds];
            let screen: *mut AnyObject = msg_send![class!(UIScreen), mainScreen];
            let scale: f64 = msg_send![screen, scale];
            let new_size = size(px(bounds.width as f32), px(bounds.height as f32));
            let new_scale = scale as f32;
            if self.bounds.get().size == new_size
                && (self.scale_factor.get() - new_scale).abs() < 0.01
            {
                return;
            }
            self.bounds.set(Bounds {
                origin: Default::default(),
                size: new_size,
            });
            self.scale_factor.set(new_scale);
            self.renderer.lock().update_drawable_size(size(
                DevicePixels((bounds.width * scale) as i32),
                DevicePixels((bounds.height * scale) as i32),
            ));
            if let Some(callback) = self.resize_callback.borrow_mut().as_mut() {
                callback(new_size, new_scale);
            }
        }
    }

    pub(crate) fn safe_area_insets(&self) -> (f32, f32, f32, f32) {
        unsafe {
            let insets: ObjcUIEdgeInsets = msg_send![self.view, safeAreaInsets];
            (
                insets.top as f32,
                insets.bottom as f32,
                insets.left as f32,
                insets.right as f32,
            )
        }
    }

    pub(crate) fn show_keyboard(&self) {
        unsafe {
            let _: () = msg_send![self.text_input_view, setKeyboardType: 0_isize];
            let _: Bool = msg_send![self.text_input_view, becomeFirstResponder];
        }
    }

    pub(crate) fn hide_keyboard(&self) {
        unsafe {
            let _: Bool = msg_send![self.text_input_view, resignFirstResponder];
        }
    }

    fn emit(&self, input: PlatformInput) {
        if let Some(callback) = self.input_callback.borrow_mut().as_mut() {
            callback(input);
        }
    }

    fn handle_touch(&self, touch: *mut AnyObject) {
        unsafe {
            let position_cg: ObjcCGPoint = msg_send![touch, locationInView: self.view];
            let phase: i64 = msg_send![touch, phase];
            let tap_count: i64 = msg_send![touch, tapCount];
            let position = point(px(position_cg.x as f32), px(position_cg.y as f32));
            self.mouse_position.set(position);
            let x = position_cg.x as f32;
            let y = position_cg.y as f32;
            let mut state = self.touch_state.get();
            match phase {
                0 => {
                    state = TouchState::Pending {
                        start_x: x,
                        start_y: y,
                    };
                }
                1 => match state {
                    TouchState::Pending { start_x, start_y } => {
                        let dx = x - start_x;
                        let dy = y - start_y;
                        if (dx * dx + dy * dy).sqrt() > SCROLL_SLOP {
                            state = TouchState::Scrolling {
                                prev_x: x,
                                prev_y: y,
                            };
                            self.emit(PlatformInput::ScrollWheel(ScrollWheelEvent {
                                position,
                                delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
                                modifiers: Modifiers::default(),
                                touch_phase: TouchPhase::Started,
                            }));
                        }
                        self.emit(PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            pressed_button: Some(MouseButton::Left),
                            modifiers: Modifiers::default(),
                        }));
                    }
                    TouchState::Scrolling { prev_x, prev_y } => {
                        let dx = x - prev_x;
                        let dy = y - prev_y;
                        state = TouchState::Scrolling {
                            prev_x: x,
                            prev_y: y,
                        };
                        self.emit(PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position,
                            delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
                            modifiers: Modifiers::default(),
                            touch_phase: TouchPhase::Moved,
                        }));
                        self.emit(PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            pressed_button: Some(MouseButton::Left),
                            modifiers: Modifiers::default(),
                        }));
                    }
                    TouchState::Idle => {}
                },
                3 | 4 => {
                    match state {
                        TouchState::Pending { start_x, start_y } => {
                            let tap_position = point(px(start_x), px(start_y));
                            self.emit(PlatformInput::MouseDown(MouseDownEvent {
                                button: MouseButton::Left,
                                position: tap_position,
                                modifiers: Modifiers::default(),
                                click_count: tap_count.max(1) as usize,
                                first_mouse: false,
                            }));
                            self.emit(PlatformInput::MouseUp(MouseUpEvent {
                                button: MouseButton::Left,
                                position: tap_position,
                                modifiers: Modifiers::default(),
                                click_count: tap_count.max(1) as usize,
                            }));
                        }
                        TouchState::Scrolling { prev_x, prev_y } => {
                            self.emit(PlatformInput::ScrollWheel(ScrollWheelEvent {
                                position,
                                delta: ScrollDelta::Pixels(point(px(x - prev_x), px(y - prev_y))),
                                modifiers: Modifiers::default(),
                                touch_phase: TouchPhase::Ended,
                            }));
                            self.emit(PlatformInput::MouseUp(MouseUpEvent {
                                button: MouseButton::Left,
                                position,
                                modifiers: Modifiers::default(),
                                click_count: 1,
                            }));
                        }
                        TouchState::Idle => {}
                    }
                    state = TouchState::Idle;
                }
                _ => {}
            }
            self.touch_state.set(state);
        }
    }

    fn handle_text_input(&self, text: *mut AnyObject) {
        let Some(text) = (unsafe { super::util::nsstring_to_string(text) }) else {
            return;
        };
        if let Some(handler) = self.input_handler.borrow_mut().as_mut() {
            handler.replace_text_in_range(None, &text);
        }
        for ch in text.chars() {
            self.emit(PlatformInput::KeyDown(crate::KeyDownEvent {
                keystroke: Keystroke {
                    modifiers: Modifiers::default(),
                    key: ch.to_string(),
                    key_char: Some(ch.to_string()),
                },
                is_held: false,
            }));
        }
    }

    fn handle_delete_backward(&self) {
        if let Some(handler) = self.input_handler.borrow_mut().as_mut() {
            handler.replace_text_in_range(None, "\u{8}");
        }
        self.emit(PlatformInput::KeyDown(crate::KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Modifiers::default(),
                key: "backspace".into(),
                key_char: None,
            },
            is_held: false,
        }));
    }
}

impl HasWindowHandle for IosWindow {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        let view = NonNull::new(self.view as *mut c_void)
            .ok_or(raw_window_handle::HandleError::Unavailable)?;
        let handle = UiKitWindowHandle::new(view);
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for IosWindow {
    fn display_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError>
    {
        let handle = UiKitDisplayHandle::new();
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(handle.into()) })
    }
}

impl PlatformWindow for IosWindow {
    fn bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }
    fn is_maximized(&self) -> bool {
        true
    }
    fn window_bounds(&self) -> WindowBounds {
        WindowBounds::Fullscreen(self.bounds.get())
    }
    fn content_size(&self) -> Size<Pixels> {
        self.bounds.get().size
    }
    fn resize(&mut self, _size: Size<Pixels>) {}
    fn scale_factor(&self) -> f32 {
        self.scale_factor.get()
    }
    fn appearance(&self) -> WindowAppearance {
        unsafe {
            let traits: *mut AnyObject = msg_send![self.view, traitCollection];
            let style: i64 = msg_send![traits, userInterfaceStyle];
            if style == 2 {
                WindowAppearance::Dark
            } else {
                WindowAppearance::Light
            }
        }
    }
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(IosDisplay::main()))
    }
    fn mouse_position(&self) -> Point<Pixels> {
        self.mouse_position.get()
    }
    fn modifiers(&self) -> Modifiers {
        Modifiers::default()
    }
    fn capslock(&self) -> Capslock {
        Capslock { on: false }
    }
    fn set_input_handler(&mut self, input_handler: PlatformInputHandler) {
        *self.input_handler.borrow_mut() = Some(input_handler);
    }
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler> {
        self.input_handler.borrow_mut().take()
    }
    fn prompt(
        &self,
        _level: PromptLevel,
        _msg: &str,
        _detail: Option<&str>,
        _answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>> {
        None
    }
    fn activate(&self) {
        if !self.window.is_null() {
            unsafe {
                let _: () = msg_send![self.window, makeKeyAndVisible];
            }
        }
    }
    fn is_active(&self) -> bool {
        true
    }
    fn is_hovered(&self) -> bool {
        false
    }
    fn set_title(&mut self, _title: &str) {}
    fn set_background_appearance(&self, _background_appearance: WindowBackgroundAppearance) {}
    fn minimize(&self) {}
    fn zoom(&self) {}
    fn toggle_fullscreen(&self) {}
    fn is_fullscreen(&self) -> bool {
        true
    }
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>) {
        *self.request_frame_callback.borrow_mut() = Some(callback);
    }
    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>) {
        *self.input_callback.borrow_mut() = Some(callback);
    }
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        *self.active_status_callback.borrow_mut() = Some(callback);
    }
    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>) {
        *self.hover_status_callback.borrow_mut() = Some(callback);
    }
    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>) {
        *self.resize_callback.borrow_mut() = Some(callback);
    }
    fn on_moved(&self, callback: Box<dyn FnMut()>) {
        *self.moved_callback.borrow_mut() = Some(callback);
    }
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>) {
        *self.should_close_callback.borrow_mut() = Some(callback);
    }
    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>) {
        *self.hit_test_callback.borrow_mut() = Some(callback);
    }
    fn on_close(&self, callback: Box<dyn FnOnce()>) {
        *self.close_callback.borrow_mut() = Some(callback);
    }
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>) {
        *self.appearance_changed_callback.borrow_mut() = Some(callback);
    }
    fn draw(&self, scene: &Scene) {
        self.renderer.lock().draw(scene);
    }
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        self.renderer.lock().sprite_atlas().clone()
    }
    fn gpu_specs(&self) -> Option<GpuSpecs> {
        Some(self.renderer.lock().gpu_specs())
    }
    fn update_ime_position(&self, _bounds: Bounds<Pixels>) {}
}
