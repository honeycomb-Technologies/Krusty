use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::anyhow;
use futures::future::BoxFuture;
use wasmtime::{component::ResourceTable, Store};
use wasmtime_wasi::{self as wasi, WasiView};

use super::wit;
use crate::extensions::ExtensionManifest;

/// State for a WASM extension instance
pub struct WasmState {
    pub manifest: Arc<ExtensionManifest>,
    pub table: ResourceTable,
    pub(crate) ctx: wasi::WasiCtx,
    pub host: Arc<super::runtime::WasmHost>,
}

std::thread_local! {
    pub static IS_WASM_THREAD: AtomicBool = const { AtomicBool::new(false) };
}

pub(super) type ExtensionCall = Box<
    dyn Send
        + for<'a> FnOnce(&'a mut wit::Extension, &'a mut Store<WasmState>) -> BoxFuture<'a, ()>,
>;

impl WasmState {
    pub fn extension_error(&self, message: String) -> anyhow::Error {
        anyhow!("Extension {}: {}", self.manifest.id, message)
    }

    /// Get the working directory for this extension
    pub fn work_dir(&self) -> PathBuf {
        self.host.work_dir.join(&self.manifest.id)
    }
}

impl WasiView for WasmState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    fn ctx(&mut self) -> &mut wasi::WasiCtx {
        &mut self.ctx
    }
}
