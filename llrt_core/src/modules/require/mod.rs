// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::env;

use once_cell::sync::Lazy;

use crate::environment;

pub mod loader;
pub mod resolver;

// added when .cjs files are imported
pub const CJS_IMPORT_PREFIX: &str = "__cjs:";
// added to force CJS imports in loader
pub const CJS_LOADER_PREFIX: &str = "__cjsm:";

pub static LLRT_PLATFORM: Lazy<String> = Lazy::new(|| {
    env::var(environment::ENV_LLRT_PLATFORM)
        .ok()
        .filter(|platform| platform == "node")
        .unwrap_or_else(|| "browser".to_string())
});

pub static COMPRESSION_DICT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compression.dict"));

include!(concat!(env!("OUT_DIR"), "/bytecode_cache.rs"));

/// Load bytecode as a module
pub fn load_bytecode_as_module<'js>(
    ctx: &rquickjs::Ctx<'js>,
    module_name: &str,
    bytecode: &[u8],
) -> rquickjs::Result<rquickjs::Module<'js>> {
    use tracing::trace;
    
    trace!("Loading bytecode as module: {}", module_name);
    
    // Attempt to load the bytecode as a module
    let bytes = loader::CustomLoader::get_module_bytecode(bytecode)
        .map_err(|e| rquickjs::Error::new_loading(format!("Failed to decompress bytecode: {}", e)))?;
    
    // Try to load as a module
    let result = unsafe { rquickjs::Module::load(ctx.clone(), &bytes) };
    
    // Return the module if successful
    if result.is_ok() {
        return result;
    }
    
    // If loading as a module fails, return the error
    result
}
