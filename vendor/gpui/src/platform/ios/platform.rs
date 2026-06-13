//! iOS Platform implementation for the crates.io GPUI 0.2.2 API.

use super::{IosDispatcher, IosDisplay, IosWindow};
use crate::platform::blade::BladeContext;
use crate::{
    Action, AnyWindowHandle, BackgroundExecutor, ClipboardItem, CursorStyle, ForegroundExecutor,
    Keymap, Menu, MenuItem, PathPromptOptions, Platform, PlatformDisplay, PlatformKeyboardLayout,
    PlatformKeyboardMapper, PlatformTextSystem, PlatformWindow, Result, Task, WindowAppearance,
    WindowParams,
};
use anyhow::anyhow;
use futures::channel::oneshot;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use parking_lot::Mutex;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

pub(crate) struct IosPlatform(Mutex<IosPlatformState>);

struct IosPlatformState {
    background_executor: BackgroundExecutor,
    foreground_executor: ForegroundExecutor,
    text_system: Arc<dyn PlatformTextSystem>,
    blade_context: Arc<BladeContext>,
    open_urls_callback: Option<Box<dyn FnMut(Vec<String>)>>,
    quit_callback: Option<Box<dyn FnMut()>>,
}

impl IosPlatform {
    pub(crate) fn new(_headless: bool) -> Self {
        let dispatcher = Arc::new(IosDispatcher);
        #[cfg(feature = "font-kit")]
        let text_system: Arc<dyn PlatformTextSystem> = Arc::new(super::IosTextSystem::new());
        #[cfg(not(feature = "font-kit"))]
        let text_system: Arc<dyn PlatformTextSystem> = Arc::new(crate::NoopTextSystem::new());

        Self(Mutex::new(IosPlatformState {
            background_executor: BackgroundExecutor::new(dispatcher.clone()),
            foreground_executor: ForegroundExecutor::new(dispatcher),
            text_system,
            blade_context: Arc::new(
                BladeContext::new().expect("failed to initialize iOS GPU context"),
            ),
            open_urls_callback: None,
            quit_callback: None,
        }))
    }
}

struct IosKeyboardLayout;

impl PlatformKeyboardLayout for IosKeyboardLayout {
    fn id(&self) -> &str {
        "ios"
    }

    fn name(&self) -> &str {
        "iOS"
    }
}

impl Platform for IosPlatform {
    fn background_executor(&self) -> BackgroundExecutor {
        self.0.lock().background_executor.clone()
    }

    fn foreground_executor(&self) -> ForegroundExecutor {
        self.0.lock().foreground_executor.clone()
    }

    fn text_system(&self) -> Arc<dyn PlatformTextSystem> {
        self.0.lock().text_system.clone()
    }

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>) {
        // Krusty's Swift shell calls into Rust after UIKit is already alive, so
        // there is no desktop-style run loop to enter here. Create GPUI state now;
        // frames are driven by gpui_ios_request_frame from CADisplayLink.
        on_finish_launching();
    }

    fn quit(&self) {
        if let Some(callback) = self.0.lock().quit_callback.as_mut() {
            callback();
        }
    }

    fn restart(&self, _binary_path: Option<PathBuf>) {}
    fn activate(&self, _ignoring_other_apps: bool) {}
    fn hide(&self) {}
    fn hide_other_apps(&self) {}
    fn unhide_other_apps(&self) {}

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        IosDisplay::all()
            .into_iter()
            .map(|display| Rc::new(display) as Rc<dyn PlatformDisplay>)
            .collect()
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        Some(Rc::new(IosDisplay::main()))
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        None
    }

    #[cfg(feature = "screen-capture")]
    fn is_screen_capture_supported(&self) -> bool {
        false
    }

    #[cfg(feature = "screen-capture")]
    fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<Result<Vec<Rc<dyn crate::ScreenCaptureSource>>>> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Err(anyhow!("screen capture is not implemented for iOS")));
        rx
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        let blade_context = self.0.lock().blade_context.clone();
        let window = Box::new(IosWindow::new(handle, options, blade_context)?);
        window.register_with_ffi();
        Ok(window)
    }

    fn window_appearance(&self) -> WindowAppearance {
        unsafe {
            let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
            let key_window: *mut AnyObject = msg_send![app, keyWindow];
            if key_window.is_null() {
                return WindowAppearance::Light;
            }
            let traits: *mut AnyObject = msg_send![key_window, traitCollection];
            let style: i64 = msg_send![traits, userInterfaceStyle];
            if style == 2 {
                WindowAppearance::Dark
            } else {
                WindowAppearance::Light
            }
        }
    }

    fn open_url(&self, url: &str) {
        unsafe {
            let ns_url_string = super::util::nsstring(url);
            let ns_url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns_url_string];
            let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
            let options: *mut AnyObject = std::ptr::null_mut();
            let completion_handler: *mut AnyObject = std::ptr::null_mut();
            let _: () = msg_send![app,
                openURL: ns_url,
                options: options,
                completionHandler: completion_handler
            ];
        }
    }

    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>) {
        self.0.lock().open_urls_callback = Some(callback);
    }

    fn register_url_scheme(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn prompt_for_paths(
        &self,
        _options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Err(anyhow!(
            "file picker is owned by the Krusty Swift shell on iOS"
        )));
        rx
    }

    fn prompt_for_new_path(
        &self,
        _directory: &Path,
        _suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Err(anyhow!(
            "save picker is owned by the Krusty Swift shell on iOS"
        )));
        rx
    }

    fn can_select_mixed_files_and_dirs(&self) -> bool {
        false
    }

    fn reveal_path(&self, _path: &Path) {}
    fn open_with_system(&self, _path: &Path) {}

    fn on_quit(&self, callback: Box<dyn FnMut()>) {
        self.0.lock().quit_callback = Some(callback);
    }

    fn on_reopen(&self, _callback: Box<dyn FnMut()>) {}
    fn set_menus(&self, _menus: Vec<Menu>, _keymap: &Keymap) {}
    fn set_dock_menu(&self, _menu: Vec<MenuItem>, _keymap: &Keymap) {}
    fn on_app_menu_action(&self, _callback: Box<dyn FnMut(&dyn Action)>) {}
    fn on_will_open_app_menu(&self, _callback: Box<dyn FnMut()>) {}
    fn on_validate_app_menu_command(&self, _callback: Box<dyn FnMut(&dyn Action) -> bool>) {}

    fn app_path(&self) -> Result<PathBuf> {
        unsafe {
            let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
            let path: *mut AnyObject = msg_send![bundle, bundlePath];
            super::util::nsstring_to_string(path)
                .map(PathBuf::from)
                .ok_or_else(|| anyhow!("failed to resolve iOS bundle path"))
        }
    }

    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        Ok(self.app_path()?.join(name))
    }

    fn set_cursor_style(&self, _style: CursorStyle) {}
    fn should_auto_hide_scrollbars(&self) -> bool {
        true
    }

    fn write_to_clipboard(&self, item: ClipboardItem) {
        unsafe {
            let pasteboard: *mut AnyObject = msg_send![class!(UIPasteboard), generalPasteboard];
            if let Some(text) = item.text() {
                let ns_text = super::util::nsstring(&text);
                let _: () = msg_send![pasteboard, setString: ns_text];
            }
        }
    }

    fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        unsafe {
            let pasteboard: *mut AnyObject = msg_send![class!(UIPasteboard), generalPasteboard];
            let text: *mut AnyObject = msg_send![pasteboard, string];
            super::util::nsstring_to_string(text).map(ClipboardItem::new_string)
        }
    }

    fn write_credentials(&self, _url: &str, _username: &str, _password: &[u8]) -> Task<Result<()>> {
        Task::ready(Err(anyhow!(
            "credentials are owned by the Krusty Swift keychain bridge on iOS"
        )))
    }

    fn read_credentials(&self, _url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        Task::ready(Err(anyhow!(
            "credentials are owned by the Krusty Swift keychain bridge on iOS"
        )))
    }

    fn delete_credentials(&self, _url: &str) -> Task<Result<()>> {
        Task::ready(Err(anyhow!(
            "credentials are owned by the Krusty Swift keychain bridge on iOS"
        )))
    }

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(IosKeyboardLayout)
    }

    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper> {
        Rc::new(crate::DummyKeyboardMapper)
    }

    fn on_keyboard_layout_change(&self, _callback: Box<dyn FnMut()>) {}
}
