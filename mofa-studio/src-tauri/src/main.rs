//! MoFA Studio: a Tauri desktop app whose React UI calls the Tauri commands in
//! [`commands`], which drive an in-process engine through the SDK's
//! [`AsyncEmbeddedEngine`] — no REST hop, no separate daemon.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod commands;
mod provision;

use std::path::PathBuf;

use commands::AppState;
use mofa_engine_sdk::{AsyncEmbeddedEngine, EngineConfig};
use tauri::Manager;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mofa_studio=info,mofa_engine_core=warn".into()),
        )
        .init();

    // Build the engine up front on Tauri's async runtime and `manage` the ready
    // state (the Builder itself is driven synchronously).
    let (state, artifacts_dir) = tauri::async_runtime::block_on(build_state()).unwrap_or_else(|e| {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    });

    tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            // Allow the webview to load generated artifacts via the asset protocol.
            // Scoping the exact directory at runtime is more reliable than the config
            // glob, which depends on `$CACHE` expanding to the real path.
            app.asset_protocol_scope().allow_directory(&artifacts_dir, true)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_capabilities,
            commands::chat_stream,
            commands::generate_image,
            commands::generate_video,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the mofa-studio Tauri application");
}

/// Provision config, boot the engine through the SDK, warm discovery. Returns the
/// state and the artifacts dir (used to scope the asset protocol).
async fn build_state() -> Result<(AppState, PathBuf), String> {
    let artifacts_dir = provision::studio_artifacts_dir()?;
    let config_path = provision::provision_config(&artifacts_dir)?;
    let engine = AsyncEmbeddedEngine::new(EngineConfig::load(Some(config_path.as_path())))
        .await
        .map_err(|e| format!("engine init failed: {e}"))?;
    engine.refresh().await;
    Ok((AppState { engine }, artifacts_dir))
}
