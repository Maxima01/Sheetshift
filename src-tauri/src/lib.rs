//! Thin desktop adapter. Conversion and persistence live in sheetshift-core.
use sheetshift_core::{
    CellEdit, ConversionPreview, ConversionRequest, HistoryEntry, PdfReviewOptions, PreviewRequest,
    ReviewedConversionRequest, Settings, Store,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

struct DesktopState {
    store: Arc<Store>,
    /// Includes outputs when persistent history is disabled.
    session_outputs: Mutex<HashSet<PathBuf>>,
}

#[derive(serde::Deserialize)]
struct DesktopConversionRequest {
    input_path: String,
    output_directory: Option<String>,
    #[serde(default)]
    preserve_formatting: bool,
}

#[derive(serde::Deserialize)]
struct DesktopReviewedConversionRequest {
    input_path: String,
    output_directory: Option<String>,
    source_fingerprint: String,
    #[serde(default)]
    pdf_options: PdfReviewOptions,
    #[serde(default)]
    edits: Vec<CellEdit>,
    #[serde(default)]
    sheet_names: Vec<String>,
    #[serde(default)]
    preserve_formatting: bool,
}

impl DesktopReviewedConversionRequest {
    fn into_core(self) -> ReviewedConversionRequest {
        let output_directory = self.output_directory.unwrap_or_else(|| {
            std::path::Path::new(&self.input_path)
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_string_lossy()
                .into_owned()
        });
        ReviewedConversionRequest {
            input_path: self.input_path,
            output_directory,
            source_fingerprint: self.source_fingerprint,
            pdf_options: self.pdf_options,
            edits: self.edits,
            sheet_names: self.sheet_names,
            preserve_formatting: self.preserve_formatting,
        }
    }
}

impl DesktopConversionRequest {
    fn into_core(self) -> ConversionRequest {
        let output_directory = self.output_directory.unwrap_or_else(|| {
            std::path::Path::new(&self.input_path)
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_string_lossy()
                .into_owned()
        });
        ConversionRequest {
            input_path: self.input_path,
            output_directory,
            preserve_formatting: self.preserve_formatting,
        }
    }
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
fn get_settings(state: State<'_, DesktopState>) -> Result<Settings, String> {
    state.store.settings().map_err(message)
}

#[tauri::command]
fn save_settings(settings: Settings, state: State<'_, DesktopState>) -> Result<Settings, String> {
    state.store.save_settings(&settings).map_err(message)?;
    state.store.settings().map_err(message)
}

#[tauri::command]
fn get_history(
    limit: Option<usize>,
    state: State<'_, DesktopState>,
) -> Result<Vec<HistoryEntry>, String> {
    state
        .store
        .history(limit.unwrap_or(200).min(1000))
        .map_err(message)
}

#[tauri::command]
fn clear_history(state: State<'_, DesktopState>) -> Result<(), String> {
    state.store.clear_history().map_err(message)
}

#[tauri::command]
fn accept_terms(state: State<'_, DesktopState>) -> Result<Settings, String> {
    state.store.accept_terms().map_err(message)?;
    state.store.settings().map_err(message)
}

#[tauri::command]
fn get_supported_formats() -> Vec<String> {
    sheetshift_core::supported_formats()
}

#[tauri::command]
async fn convert_file(
    request: DesktopConversionRequest,
    state: State<'_, DesktopState>,
) -> Result<HistoryEntry, String> {
    let store = Arc::clone(&state.store);
    let request = request.into_core();
    // Parsing is CPU/disk intensive: never block Tauri's UI thread.
    let entry = tauri::async_runtime::spawn_blocking(move || store.convert(&request))
        .await
        .map_err(message)?
        .map_err(message)?;
    if let Some(path) = &entry.output_path {
        if entry.status == "success" {
            let canonical = std::fs::canonicalize(path).map_err(message)?;
            state
                .session_outputs
                .lock()
                .map_err(message)?
                .insert(canonical);
        }
    }
    Ok(entry)
}

#[tauri::command]
async fn preview_file(request: PreviewRequest) -> Result<ConversionPreview, String> {
    tauri::async_runtime::spawn_blocking(move || sheetshift_core::preview_file(&request))
        .await
        .map_err(message)?
        .map_err(message)
}

#[tauri::command]
async fn convert_reviewed_file(
    request: DesktopReviewedConversionRequest,
    state: State<'_, DesktopState>,
) -> Result<HistoryEntry, String> {
    let store = Arc::clone(&state.store);
    let request = request.into_core();
    let entry = tauri::async_runtime::spawn_blocking(move || store.convert_reviewed(&request))
        .await
        .map_err(message)?
        .map_err(message)?;
    if let Some(path) = &entry.output_path {
        if entry.status == "success" {
            let canonical = std::fs::canonicalize(path).map_err(message)?;
            state
                .session_outputs
                .lock()
                .map_err(message)?
                .insert(canonical);
        }
    }
    Ok(entry)
}

#[tauri::command]
fn open_output(
    path: String,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<(), String> {
    let canonical = std::fs::canonicalize(&path).map_err(|_| {
        "The workbook has been moved or deleted. Check its saved location.".to_string()
    })?;
    if !canonical.is_file() || canonical.extension().and_then(|e| e.to_str()) != Some("xlsx") {
        return Err("Only converted Excel workbooks can be opened.".into());
    }
    let in_session = state
        .session_outputs
        .lock()
        .map_err(message)?
        .contains(&canonical);
    let in_history = if in_session {
        false
    } else {
        state
            .store
            .history(1000)
            .map_err(message)?
            .iter()
            .any(|entry| {
                entry.status == "success"
                    && entry
                        .output_path
                        .as_ref()
                        .and_then(|p| std::fs::canonicalize(p).ok())
                        .as_ref()
                        == Some(&canonical)
            })
    };
    if !in_session && !in_history {
        return Err("This workbook is not a known conversion output.".into());
    }
    app.opener()
        .open_path(canonical.to_string_lossy().into_owned(), None::<&str>)
        .map_err(message)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let directory = sheetshift_core::default_data_dir()?;
            std::fs::create_dir_all(&directory)?;
            let store = Store::open(directory.join("sheetshift.sqlite3"))?;
            app.manage(DesktopState {
                store: Arc::new(store),
                session_outputs: Mutex::new(HashSet::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            get_history,
            clear_history,
            accept_terms,
            get_supported_formats,
            preview_file,
            convert_file,
            convert_reviewed_file,
            open_output
        ])
        .run(tauri::generate_context!())
        .expect("Sheetshift could not start. Check that your local data directory is writable.");
}
