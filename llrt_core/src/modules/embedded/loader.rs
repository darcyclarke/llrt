// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::{env, fs, io, result::Result as StdResult};

use once_cell::sync::Lazy;
use rquickjs::{loader::Loader, Ctx, Error, Module, Object, Result};
use tracing::trace;
use zstd::{bulk::Decompressor, dict::DecoderDictionary};

use crate::bytecode::{
    BYTECODE_COMPRESSED, BYTECODE_FILE_EXT, BYTECODE_UNCOMPRESSED, BYTECODE_VERSION,
    SIGNATURE_LENGTH,
};

use super::{BYTECODE_CACHE, CJS_IMPORT_PREFIX, CJS_LOADER_PREFIX, COMPRESSION_DICT};

static DECOMPRESSOR_DICT: Lazy<DecoderDictionary> =
    Lazy::new(|| DecoderDictionary::copy(COMPRESSION_DICT));

#[cfg(feature = "lambda")]
include!(concat!(env!("OUT_DIR"), "/sdk_client_endpoints.rs"));

#[derive(Debug, Default)]
pub struct EmbeddedLoader;

impl EmbeddedLoader {
    pub fn load_bytecode_module<'js>(ctx: Ctx<'js>, buf: &[u8]) -> Result<Module<'js>> {
        let bytes = Self::get_module_bytecode(buf)?;
        unsafe { Module::load(ctx, &bytes) }
    }

    #[inline]
    pub fn uncompressed_size(input: &[u8]) -> StdResult<(usize, &[u8]), io::Error> {
        let size = input.get(..4).ok_or(io::ErrorKind::InvalidInput)?;
        let size: &[u8; 4] = size.try_into().map_err(|_| io::ErrorKind::InvalidInput)?;
        let uncompressed_size = u32::from_le_bytes(*size) as usize;
        let rest = &input[4..];
        Ok((uncompressed_size, rest))
    }

    fn get_module_bytecode(input: &[u8]) -> Result<Vec<u8>> {
        let (_, compressed, input) = Self::get_bytecode_signature(input)?;

        if compressed {
            let (size, input) = Self::uncompressed_size(input)?;
            let mut buf = Vec::with_capacity(size);
            let mut decompressor = Decompressor::with_prepared_dictionary(&DECOMPRESSOR_DICT)?;
            decompressor.decompress_to_buffer(input, &mut buf)?;
            return Ok(buf);
        }

        Ok(input.to_vec())
    }

    pub fn get_bytecode_signature(input: &[u8]) -> StdResult<(&[u8], bool, &[u8]), io::Error> {
        let raw_signature = input
            .get(..SIGNATURE_LENGTH)
            .ok_or(io::Error::new::<String>(
                io::ErrorKind::InvalidInput,
                "Invalid bytecode signature length".into(),
            ))?;

        let (last, signature) = raw_signature.split_last().unwrap();

        if signature != BYTECODE_VERSION.as_bytes() {
            return Err(io::Error::new::<String>(
                io::ErrorKind::InvalidInput,
                "Invalid bytecode version".into(),
            ));
        }

        let mut compressed = None;
        if *last == BYTECODE_COMPRESSED {
            compressed = Some(true)
        } else if *last == BYTECODE_UNCOMPRESSED {
            compressed = Some(false)
        }

        let rest = &input[SIGNATURE_LENGTH..];
        Ok((
            signature,
            compressed.ok_or(io::Error::new::<String>(
                io::ErrorKind::InvalidInput,
                "Invalid bytecode signature".into(),
            ))?,
            rest,
        ))
    }

    fn normalize_name(name: &str) -> (bool, bool, &str, &str) {
        if !name.starts_with("__") {
            // If name doesn't start with "__", return defaults
            return (false, false, name, name);
        }

        if let Some(cjs_path) = name.strip_prefix(CJS_IMPORT_PREFIX) {
            // If it starts with CJS_IMPORT_PREFIX, mark as from_cjs_import
            return (true, false, name, cjs_path);
        }

        if let Some(cjs_path) = name.strip_prefix(CJS_LOADER_PREFIX) {
            // If it starts with CJS_LOADER_PREFIX, mark as is_cjs
            return (false, true, cjs_path, cjs_path);
        }

        // Default return if no prefixes match
        (false, false, name, name)
    }

    fn load_module<'js>(name: &str, ctx: &Ctx<'js>) -> Result<(Module<'js>, Option<String>)> {
        let ctx = ctx.clone();

        let (_, _, normalized_name, path) = Self::normalize_name(name);

        // Try to find in bytecode cache
        if let Ok(cache) = BYTECODE_CACHE.read() {
            if let Some(bytes) = cache.get(path) {
                #[cfg(feature = "lambda")]
                init_client_connection(&ctx, path)?;

                trace!("Loading embedded module: {}\n", path);

                return Ok((Self::load_bytecode_module(ctx, bytes)?, Some(path.into())));
            }
        }

        let bytes = std::fs::read(path)?;
        let bytes: &[u8] = &bytes;

        if normalized_name.ends_with(BYTECODE_FILE_EXT) {
            trace!("Loading binary module: {}\n", path);
            return Ok((Self::load_bytecode_module(ctx, bytes)?, Some(path.into())));
        }

        Err(Error::new_loading_message(path, "unable to load"))
    }
}

impl Loader for EmbeddedLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str) -> Result<Module<'js>> {
        let (module, url) = Self::load_module(name, ctx)?;
        if let Some(url) = url {
            let meta: Object = module.meta()?;
            meta.prop("url", url)?;
        }

        Ok(module)
    }
}

/// Load embedded bytecode from self-contained executables
#[cfg(not(feature = "lambda"))]
pub fn load_embedded_executable_bytecode() -> StdResult<(), io::Error> {
    use std::path::PathBuf;

    let executable_path = env::current_exe()
        .unwrap_or_else(|_| PathBuf::from(env::args().next().unwrap_or_default()));

    trace!(
        "Checking if {} is a self-contained executable",
        executable_path.display()
    );

    // Read the last 4 bytes to check for signature
    if let Ok(exe_content) = fs::read(&executable_path) {
        const MARKER: &[u8] = b"LLRT_EXE";
        
        if exe_content.len() > MARKER.len() + 8 {
            let marker_pos = exe_content.len() - MARKER.len();
            
            if &exe_content[marker_pos..] == MARKER {
                trace!("Found LLRT_EXE marker at position {}", marker_pos);

                // Read the bytecode size (8 bytes before marker)
                let size_start = marker_pos - 8;
                let size_bytes = &exe_content[size_start..marker_pos];
                let bytecode_size = u64::from_le_bytes([
                    size_bytes[0],
                    size_bytes[1],
                    size_bytes[2],
                    size_bytes[3],
                    size_bytes[4],
                    size_bytes[5],
                    size_bytes[6],
                    size_bytes[7],
                ]) as usize;

                trace!("Bytecode size from footer: {} bytes", bytecode_size);

                // Validate the size
                if bytecode_size > 0 && bytecode_size < exe_content.len() {
                    let bytecode_start = marker_pos - 8 - bytecode_size;
                    trace!("Bytecode starts at offset {}", bytecode_start);

                    // Extract the compressed bytecode
                    let compressed_bytecode = &exe_content[bytecode_start..bytecode_start + bytecode_size];
                    
                    // Decompress the bytecode
                    match EmbeddedLoader::get_module_bytecode(compressed_bytecode) {
                        Ok(bytecode) => {
                            trace!("Successfully decompressed bytecode");
                            
                            // Store in the global bytecode cache with "main" as the key
                            if let Ok(mut cache) = BYTECODE_CACHE.write() {
                                // Add both with and without extension for compatibility
                                cache.insert("main".to_string(), bytecode.clone());
                                cache.insert("main.js".to_string(), bytecode);
                                trace!("Added embedded bytecode to cache as 'main'");
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            trace!("Failed to decompress bytecode: {:?}", e);
                        }
                    }
                }
            }
        }
    }

    // Not a self-contained executable or failed to load
    Ok(())
}

#[cfg(feature = "lambda")]
fn init_client_connection(ctx: &Ctx<'_>, specifier: &str) -> Result<()> {
    use std::{env, time::Instant};

    use http_body_util::BodyExt;
    use rquickjs::qjs;

    use crate::libs::utils::result::ResultExt;
    use crate::modules::http::HTTP_CLIENT;
    use crate::runtime_client::{check_client_inited, mark_client_inited};

    if let Some(sdk_import) = specifier.strip_prefix("@aws-sdk/") {
        let client_name = sdk_import.trim_start_matches("client-");
        if let Some(endpoint) = SDK_CLIENT_ENDPOINTS.get(client_name) {
            let endpoint = if endpoint.is_empty() {
                client_name
            } else {
                endpoint
            };

            let rt = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
            let rt_ptr = rt as usize; //hack to move, is safe since runtime is still alive in spawn

            if !check_client_inited(rt, endpoint) {
                let client = HTTP_CLIENT.as_ref().or_throw(ctx)?;

                trace!("Started client init {}", client_name);
                let region = env::var("AWS_REGION").unwrap();

                let url = ["https://", endpoint, ".", &region, ".amazonaws.com/sping"].concat();

                tokio::task::spawn(async move {
                    let start = Instant::now();

                    if let Ok(url) = url.parse() {
                        if let Ok(mut res) = client.get(url).await {
                            if let Ok(res) = res.body_mut().collect().await {
                                let _ = res;

                                mark_client_inited(rt_ptr as _);

                                trace!("Client connection initialized in {:?}", start.elapsed());
                            }
                        }
                    }
                });
            }
        }
    }

    Ok(())
}
