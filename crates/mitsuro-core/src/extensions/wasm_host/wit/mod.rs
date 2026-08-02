//! Multi-version WIT bindings for Zed extension compatibility
//!
//! Each version module contains bindgen! generated code for that API version.
//! Extensions are instantiated with the appropriate version based on their
//! declared lib.version in extension.toml.

mod since_v0_0_1;
mod since_v0_0_4;
mod since_v0_0_6;
mod since_v0_1_0;
mod since_v0_2_0;
mod since_v0_3_0;
mod since_v0_4_0;
mod since_v0_5_0;
mod since_v0_6_0;
mod since_v0_8_0;

use crate::extensions::types::{LanguageServerName, WorktreeDelegate};
use crate::extensions::wasm_host::{wasm_engine, WasmState};
use anyhow::{Context as _, Result};
use semver::Version;
use since_v0_8_0 as latest;
use std::sync::Arc;
use wasmtime::{
    component::{Component, Linker, Resource},
    Store,
};

// Re-export the latest Command type for external use
pub use latest::Command;

pub fn new_linker(
    f: impl Fn(&mut Linker<WasmState>, fn(&mut WasmState) -> &mut WasmState) -> Result<()>,
) -> Result<Linker<WasmState>> {
    let mut linker = Linker::new(&wasm_engine());
    wasmtime_wasi::add_to_linker_async(&mut linker)?;
    f(&mut linker, wasi_view)?;
    Ok(linker)
}

fn wasi_view(state: &mut WasmState) -> &mut WasmState {
    state
}

/// Extension enum that wraps all supported API versions
pub enum Extension {
    V0_8_0(since_v0_8_0::Extension),
    V0_6_0(since_v0_6_0::Extension),
    V0_5_0(since_v0_5_0::Extension),
    V0_4_0(since_v0_4_0::Extension),
    V0_3_0(since_v0_3_0::Extension),
    V0_2_0(since_v0_2_0::Extension),
    V0_1_0(since_v0_1_0::Extension),
    V0_0_6(since_v0_0_6::Extension),
    V0_0_4(since_v0_0_4::Extension),
    V0_0_1(since_v0_0_1::Extension),
}

impl Extension {
    pub async fn instantiate_async(
        store: &mut Store<WasmState>,
        version: &Version,
        component: &Component,
    ) -> Result<Self> {
        tracing::debug!("Instantiating extension with API version {}", version);
        if *version >= latest::MIN_VERSION {
            tracing::debug!("Using v0.8.0 (latest) linker");
            let extension =
                latest::Extension::instantiate_async(store, component, latest::linker())
                    .await
                    .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_8_0(extension))
        } else if *version >= since_v0_6_0::MIN_VERSION {
            tracing::debug!("Using v0.6.0 linker");
            let extension = since_v0_6_0::Extension::instantiate_async(
                store,
                component,
                since_v0_6_0::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_6_0(extension))
        } else if *version >= since_v0_5_0::MIN_VERSION {
            tracing::debug!("Using v0.5.0 linker");
            let extension = since_v0_5_0::Extension::instantiate_async(
                store,
                component,
                since_v0_5_0::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_5_0(extension))
        } else if *version >= since_v0_4_0::MIN_VERSION {
            tracing::debug!("Using v0.4.0 linker");
            let extension = since_v0_4_0::Extension::instantiate_async(
                store,
                component,
                since_v0_4_0::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_4_0(extension))
        } else if *version >= since_v0_3_0::MIN_VERSION {
            tracing::debug!("Using v0.3.0 linker");
            let extension = since_v0_3_0::Extension::instantiate_async(
                store,
                component,
                since_v0_3_0::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_3_0(extension))
        } else if *version >= since_v0_2_0::MIN_VERSION {
            tracing::debug!("Using v0.2.0 linker");
            let extension = since_v0_2_0::Extension::instantiate_async(
                store,
                component,
                since_v0_2_0::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_2_0(extension))
        } else if *version >= since_v0_1_0::MIN_VERSION {
            tracing::debug!("Using v0.1.0 linker");
            let extension = since_v0_1_0::Extension::instantiate_async(
                store,
                component,
                since_v0_1_0::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_1_0(extension))
        } else if *version >= since_v0_0_6::MIN_VERSION {
            tracing::debug!("Using v0.0.6 linker");
            let extension = since_v0_0_6::Extension::instantiate_async(
                store,
                component,
                since_v0_0_6::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_0_6(extension))
        } else if *version >= since_v0_0_4::MIN_VERSION {
            tracing::debug!("Using v0.0.4 linker");
            let extension = since_v0_0_4::Extension::instantiate_async(
                store,
                component,
                since_v0_0_4::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_0_4(extension))
        } else {
            tracing::debug!("Using v0.0.1 linker");
            let extension = since_v0_0_1::Extension::instantiate_async(
                store,
                component,
                since_v0_0_1::linker(),
            )
            .await
            .context("failed to instantiate wasm extension")?;
            Ok(Self::V0_0_1(extension))
        }
    }

    pub async fn call_init_extension(&self, store: &mut Store<WasmState>) -> Result<()> {
        match self {
            Extension::V0_8_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_6_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_5_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_4_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_3_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_2_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_1_0(ext) => ext.call_init_extension(store).await,
            Extension::V0_0_6(ext) => ext.call_init_extension(store).await,
            Extension::V0_0_4(ext) => ext.call_init_extension(store).await,
            Extension::V0_0_1(ext) => ext.call_init_extension(store).await,
        }
    }

    pub async fn call_language_server_command(
        &self,
        store: &mut Store<WasmState>,
        language_server_id: &LanguageServerName,
        resource: Resource<Arc<dyn WorktreeDelegate>>,
    ) -> Result<Result<Command, String>> {
        match self {
            Extension::V0_8_0(ext) => {
                ext.call_language_server_command(store, &language_server_id.0, resource)
                    .await
            }
            Extension::V0_6_0(ext) => {
                ext.call_language_server_command(store, &language_server_id.0, resource)
                    .await
            }
            Extension::V0_5_0(ext) => {
                ext.call_language_server_command(store, &language_server_id.0, resource)
                    .await
            }
            Extension::V0_4_0(ext) => {
                ext.call_language_server_command(store, &language_server_id.0, resource)
                    .await
            }
            Extension::V0_3_0(ext) => {
                ext.call_language_server_command(store, &language_server_id.0, resource)
                    .await
            }
            Extension::V0_2_0(ext) => ext
                .call_language_server_command(store, &language_server_id.0, resource)
                .await
                .map(|r| r.map(Into::into)),
            Extension::V0_1_0(ext) => ext
                .call_language_server_command(store, &language_server_id.0, resource)
                .await
                .map(|r| r.map(Into::into)),
            Extension::V0_0_6(ext) => ext
                .call_language_server_command(store, &language_server_id.0, resource)
                .await
                .map(|r| r.map(Into::into)),
            Extension::V0_0_4(ext) => ext
                .call_language_server_command(
                    store,
                    &since_v0_0_4::LanguageServerConfig {
                        name: language_server_id.0.to_string(),
                        language_name: String::new(),
                    },
                    resource,
                )
                .await
                .map(|r| r.map(Into::into)),
            Extension::V0_0_1(ext) => ext
                .call_language_server_command(
                    store,
                    &since_v0_0_1::LanguageServerConfig {
                        name: language_server_id.0.to_string(),
                        language_name: String::new(),
                    },
                    resource,
                )
                .await
                .map(|r| r.map(Into::into)),
        }
    }
}
