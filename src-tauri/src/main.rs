// No console window behind the app in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    borrow::Cow,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

use tauri::{
    http,
    ipc::{InvokeBody, Request},
    AppHandle, Emitter, Manager, State, WebviewWindow,
};

// ---------------------------------------------------------------- disk files
//
// The page reads models, .mtl files and textures through the `fs` scheme
// (http://fs.localhost/C:/dir/model.fbx on Windows).

/// Where textures usually live when they are not next to the model.
const TEXTURE_DIRS: [&str; 13] = [
    "textures", "texture", "tex", "maps", "materials", "images",
    "../textures", "../texture", "../tex", "../maps", "../materials", "../images", "..",
];

/// A file referenced by a model. If it is not where the model says, look in
/// the usual texture folders around it.
fn resolve_disk_file(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let (dir, name) = (path.parent()?, path.file_name()?);
    TEXTURE_DIRS
        .iter()
        .map(|sub| dir.join(sub).join(name))
        .find(|candidate| candidate.is_file())
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let value = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok());
            if let Some(value) = value {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mime_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

fn disk_response(request: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let decoded = percent_decode(request.uri().path());
    // "/C:/dir/file" -> "C:/dir/file"; a UNC path keeps its two leading slashes.
    let path = decoded.strip_prefix('/').unwrap_or(&decoded);
    let found = resolve_disk_file(Path::new(path)).and_then(|file| {
        let data = fs::read(&file).ok()?;
        Some((mime_for(&file), data))
    });
    let builder = http::Response::builder()
        // The page lives on another origin (tauri.localhost).
        .header("Access-Control-Allow-Origin", "*")
        // A model that is re-exported must not come back from a cache.
        .header("Cache-Control", "no-store");
    match found {
        Some((mime, data)) => builder.header("Content-Type", mime).body(data),
        None => builder.status(404).body(b"Not found".to_vec()),
    }
    .expect("valid response")
}

// ---------------------------------------------------------------- launch
fn is_model(arg: &str) -> bool {
    let lower = arg.to_ascii_lowercase();
    lower.ends_with(".fbx") || lower.ends_with(".obj")
}

fn models_from_args(args: &[String], cwd: &Path) -> Vec<String> {
    args.iter()
        .skip(1)
        .filter(|arg| is_model(arg))
        .map(|arg| cwd.join(arg))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// What the app was started with: model files as absolute paths, and the
/// `--switches` unchanged. The page turns them into its view settings.
#[tauri::command]
fn launch_args() -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut out = models_from_args(&args, &cwd);
    out.extend(args.iter().skip(1).filter(|arg| arg.starts_with("--")).cloned());
    out
}

// ---------------------------------------------------------------- file watching
//
// The page hands over the files of its tabs; a background thread polls them
// and reports a file once it has changed and then been quiet for a moment
// (exporters write in several steps).

type Stamp = Option<(SystemTime, u64)>;

struct Seen {
    stamp: Stamp,
    changed_at: Option<Instant>,
}

#[derive(Default)]
struct Watched(Mutex<HashMap<String, Seen>>);

fn stamp_of(path: &str) -> Stamp {
    let meta = fs::metadata(path).ok()?;
    if meta.len() == 0 {
        return None; // being rewritten right now
    }
    Some((meta.modified().ok()?, meta.len()))
}

#[tauri::command]
fn watch_files(paths: Vec<String>, watched: State<'_, Arc<Watched>>) {
    let mut files = watched.0.lock().unwrap();
    files.retain(|path, _| paths.contains(path));
    for path in paths {
        if !files.contains_key(&path) {
            let stamp = stamp_of(&path);
            files.insert(path, Seen { stamp, changed_at: None });
        }
    }
}

fn start_watcher(app: AppHandle, watched: Arc<Watched>) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(250));
        let mut changed = Vec::new();
        {
            let mut files = watched.0.lock().unwrap();
            for (path, seen) in files.iter_mut() {
                let now = stamp_of(path);
                if now.is_some() && now != seen.stamp {
                    seen.stamp = now;
                    seen.changed_at = Some(Instant::now());
                } else if seen.changed_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(400)) {
                    seen.changed_at = None;
                    changed.push(path.clone());
                }
            }
        }
        for path in changed {
            let _ = app.emit("file-changed", path);
        }
    });
}

// ---------------------------------------------------------------- screenshots
fn raw_body<'a>(request: &'a Request<'_>) -> Result<&'a [u8], String> {
    match request.body() {
        InvokeBody::Raw(bytes) => Ok(bytes),
        _ => Err("expected binary data".into()),
    }
}

fn header<'a>(request: &'a Request<'_>, name: &str) -> Result<&'a str, String> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| format!("missing header {name}"))
}

/// Body: RGBA pixels, top row first. Headers: x-width, x-height.
#[tauri::command]
fn copy_image(request: Request<'_>) -> Result<(), String> {
    let bytes = raw_body(&request)?;
    let width: usize = header(&request, "x-width")?.parse().map_err(|_| "bad width")?;
    let height: usize = header(&request, "x-height")?.parse().map_err(|_| "bad height")?;
    if bytes.len() != width * height * 4 {
        return Err("pixel data does not match the image size".into());
    }
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_image(arboard::ImageData { width, height, bytes: Cow::Borrowed(bytes) })
        .map_err(|e| e.to_string())
}

/// Body: the file's bytes. Header: x-path (percent-encoded).
#[tauri::command]
fn save_file(request: Request<'_>) -> Result<(), String> {
    let path = percent_decode(header(&request, "x-path")?);
    fs::write(&path, raw_body(&request)?).map_err(|e| format!("{path}: {e}"))
}

// ---------------------------------------------------------------- window
#[tauri::command]
fn set_title(window: WebviewWindow, title: String) {
    let _ = window.set_title(&title);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    let builder = tauri::Builder::default();

    // Started again (for example "Open with" in Explorer): hand the files to the
    // window that is already open, where they become new tabs. Debug builds skip
    // this so they can run next to an installed copy.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
        let Some(window) = app.get_webview_window("main") else { return };
        let models = models_from_args(&argv, Path::new(&cwd));
        if !models.is_empty() {
            let _ = window.emit("open-files", models);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }));

    builder
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("fs", |_context, request| disk_response(&request))
        .invoke_handler(tauri::generate_handler![
            launch_args,
            watch_files,
            copy_image,
            save_file,
            set_title,
            quit
        ])
        .setup(|app| {
            let watched = Arc::new(Watched::default());
            app.manage(watched.clone());
            start_watcher(app.handle().clone(), watched);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Model Viewer");
}
