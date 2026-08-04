use crate::util::game_resolver::resolve_installed_game;
use crate::steamless::{
    SteamlessRunRequest, SteamlessRunResult, SteamlessRunner, SteamlessToolLocator,
};
use std::path::PathBuf;
use tauri_plugin_dialog::{DialogExt, FilePath};

const STEAMLESS_TIMEOUT_SECONDS: u64 = 120;

#[tauri::command]
pub async fn pick_and_run_steamless(
    app: tauri::AppHandle,
    app_id: u32,
) -> Result<SteamlessRunResult, String> {
    let game = resolve_installed_game(&app, app_id)?;
    let game_root = PathBuf::from(&game.game_path);

    let selected_file = app
        .dialog()
        .file()
        .set_title(format!("Select executable for {}", game.name))
        .set_directory(&game_root)
        .add_filter("Executables", &["exe"])
        .blocking_pick_file();

    let Some(file_path) = selected_file else {
        return Ok(SteamlessRunResult {
            success: false,
            cancelled: true,
            message: "Steamless cancelled.".to_string(),
            exe_path: None,
            backup_path: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        });
    };

    let exe_path = file_path_to_path_buf(file_path)?;
    let tool = SteamlessToolLocator::new(app.clone()).locate()?;

    tauri::async_runtime::spawn_blocking(move || {
        SteamlessRunner::new(tool).run(SteamlessRunRequest {
            exe_path,
            game_root,
            timeout_seconds: STEAMLESS_TIMEOUT_SECONDS,
        })
    })
    .await
    .map_err(|e| format!("Steamless worker failed: {}", e))?
}

fn file_path_to_path_buf(file_path: FilePath) -> Result<PathBuf, String> {
    file_path.into_path().map_err(|e| {
        format!(
            "Selected executable path is not a local filesystem path: {}",
            e
        )
    })
}
