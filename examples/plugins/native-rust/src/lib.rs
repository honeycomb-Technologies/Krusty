use std::ffi::{c_char, c_void, CString};

const MITSURO_NATIVE_PLUGIN_ABI_VERSION: u32 = 1;
const MITSURO_NATIVE_EVENT_KEY: u32 = 1;
const MITSURO_NATIVE_EVENT_RESULT_IGNORED: u32 = 0;
const MITSURO_NATIVE_EVENT_RESULT_CONSUMED: u32 = 1;

#[repr(C)]
pub struct MitsuroNativePluginV1 {
    pub abi_version: u32,
    pub create: Option<unsafe extern "C" fn() -> *mut c_void>,
    pub destroy: Option<unsafe extern "C" fn(instance: *mut c_void)>,
    pub on_activate: Option<unsafe extern "C" fn(instance: *mut c_void)>,
    pub on_deactivate: Option<unsafe extern "C" fn(instance: *mut c_void)>,
    pub tick: Option<unsafe extern "C" fn(instance: *mut c_void) -> bool>,
    pub render_text: Option<
        unsafe extern "C" fn(
            instance: *mut c_void,
            width: u16,
            height: u16,
            sink: unsafe extern "C" fn(userdata: *mut c_void, text: *const c_char),
            userdata: *mut c_void,
        ),
    >,
    pub handle_event:
        Option<unsafe extern "C" fn(instance: *mut c_void, event: MitsuroNativeEvent) -> u32>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MitsuroNativeEvent {
    pub kind: u32,
    pub key_code: u32,
    pub modifiers: u32,
}

struct DemoState {
    ticks: u64,
    key_presses: u64,
}

static PLUGIN: MitsuroNativePluginV1 = MitsuroNativePluginV1 {
    abi_version: MITSURO_NATIVE_PLUGIN_ABI_VERSION,
    create: Some(create),
    destroy: Some(destroy),
    on_activate: None,
    on_deactivate: None,
    tick: Some(tick),
    render_text: Some(render_text),
    handle_event: Some(handle_event),
};

#[no_mangle]
pub extern "C" fn mitsuro_plugin_entry() -> *const MitsuroNativePluginV1 {
    &PLUGIN
}

unsafe extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(DemoState {
        ticks: 0,
        key_presses: 0,
    }))
    .cast()
}

unsafe extern "C" fn destroy(instance: *mut c_void) {
    if !instance.is_null() {
        drop(Box::from_raw(instance.cast::<DemoState>()));
    }
}

unsafe extern "C" fn tick(instance: *mut c_void) -> bool {
    let Some(state) = instance.cast::<DemoState>().as_mut() else {
        return false;
    };
    state.ticks = state.ticks.wrapping_add(1);
    state.ticks % 30 == 0
}

unsafe extern "C" fn render_text(
    instance: *mut c_void,
    width: u16,
    height: u16,
    sink: unsafe extern "C" fn(userdata: *mut c_void, text: *const c_char),
    userdata: *mut c_void,
) {
    let Some(state) = instance.cast::<DemoState>().as_ref() else {
        emit(sink, userdata, "Native Rust Demo: missing state");
        return;
    };

    emit(sink, userdata, "Native Rust Demo");
    emit(sink, userdata, "================");
    emit(sink, userdata, &format!("area: {width}x{height}"));
    emit(sink, userdata, &format!("ticks: {}", state.ticks));
    emit(
        sink,
        userdata,
        &format!("key presses: {}", state.key_presses),
    );
    emit(sink, userdata, "");
    emit(
        sink,
        userdata,
        "Edit src/lib.rs, rebuild, then run /plugins reload native-rust-demo.",
    );
}

unsafe extern "C" fn handle_event(instance: *mut c_void, event: MitsuroNativeEvent) -> u32 {
    if event.kind != MITSURO_NATIVE_EVENT_KEY {
        return MITSURO_NATIVE_EVENT_RESULT_IGNORED;
    }
    let Some(state) = instance.cast::<DemoState>().as_mut() else {
        return MITSURO_NATIVE_EVENT_RESULT_IGNORED;
    };
    state.key_presses = state.key_presses.wrapping_add(1);
    MITSURO_NATIVE_EVENT_RESULT_CONSUMED
}

unsafe fn emit(
    sink: unsafe extern "C" fn(userdata: *mut c_void, text: *const c_char),
    userdata: *mut c_void,
    text: &str,
) {
    let c_string = CString::new(text).unwrap_or_else(|_| CString::new("<invalid text>").unwrap());
    sink(userdata, c_string.as_ptr());
}
