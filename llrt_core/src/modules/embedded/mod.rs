// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::env;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use once_cell::sync::Lazy;
use rquickjs::{Ctx, Function, Result};

use self::resolver::embedded_resolve;

pub mod loader;
pub mod resolver;

// added when .cjs files are imported
const CJS_IMPORT_PREFIX: &str = "__cjs:";
// added to force CJS imports in loader
const CJS_LOADER_PREFIX: &str = "__cjsm:";

pub static COMPRESSION_DICT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compression.dict"));

// Create a rename of BYTECODE_CACHE to STATIC_BYTECODE_CACHE before including the generated file
// This allows us to use BYTECODE_CACHE as our mutable version while keeping the build-generated static one
mod generated {
    include!(concat!(env!("OUT_DIR"), "/bytecode_cache.rs"));
}
use generated::BYTECODE_CACHE as STATIC_BYTECODE_CACHE;

// Create a global mutable bytecode cache that starts with the static cache
pub static BYTECODE_CACHE: Lazy<Arc<RwLock<HashMap<String, Vec<u8>>>>> = Lazy::new(|| {
    let mut cache = HashMap::new();
    
    // Populate with static bytecode cache from build time
    for (key, value) in STATIC_BYTECODE_CACHE.entries() {
        cache.insert(key.to_string(), value.to_vec());
    }
    
    Arc::new(RwLock::new(cache))
});

pub fn init(ctx: &Ctx) -> Result<()> {
    let globals = ctx.globals();

    let embedded_hook = Function::new(ctx.clone(), move |x: String, y: String| {
        embedded_resolve(&x, &y).map(|res| res.into_owned())
    })?;

    globals.set("__embedded_hook", embedded_hook)?;

    // Check for embedded bytecode in self-contained executables
    #[cfg(not(feature = "lambda"))]
    {
        if let Ok(()) = crate::modules::embedded::loader::load_embedded_executable_bytecode() {
            // Embedded bytecode was loaded into the cache
        }
    }

    Ok(())
}
