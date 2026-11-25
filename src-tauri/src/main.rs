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

use sha2::{Digest, Sha256};
use tauri::{
    http,
    ipc::{InvokeBody, Request},
    AppHandle, Emitter, Manager, State, WebviewWindow,
};
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        System::{
            Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED},
            Registry::{
                RegCloseKey, RegCreateKeyExW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD,
                REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
            },
        },
        UI::Shell::{
            IShellItem, ISharedBitmap, IThumbnailCache, SHChangeNotify, SHCreateItemFromParsingName, SHCNE_ASSOCCHANGED,
            SHCNE_UPDATEITEM, SHCNF_FLAGS, SHCNF_FLUSHNOWAIT, SHCNF_IDLIST, SHCNF_PATHW, WTS_FORCEEXTRACTION,
        },
    },
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

// ---------------------------------------------------------------- thumbnails
//
// Explorer gets a model's thumbnail from ModelViewerThumbnail.dll
// (src-tauri/thumbnail), which answers from the cache the page fills here:
// one PNG per file, named by the file's content hash so it survives moves and
// copies and goes stale when the file is re-exported.

fn thumbnail_dir() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(local).join("ModelViewer").join("thumbnails"))
}

/// SHA-256 of the file's bytes, first 16 bytes as hex. Must match
/// `content_key` in src-tauri/thumbnail/src/lib.rs.
fn thumbnail_key(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Ok(Sha256::digest(&bytes)[..16].iter().map(|b| format!("{b:02x}")).collect())
}

/// The shell's own thumbnail cache (CLSID_LocalThumbnailCache).
const LOCAL_THUMBNAIL_CACHE: GUID = GUID::from_u128(0x50ef4544_ac9f_4a8e_b21b_8a26180db13f);
/// The sizes Explorer's views and preview pane ask for.
const SHELL_THUMBNAIL_SIZES: [u32; 5] = [48, 96, 256, 768, 1280];

/// Explorer remembers per file what the provider last answered, a thumbnail or
/// "none", until the file itself changes; a change notice alone does not make
/// it ask again. Forcing its cache to extract anew replaces that memory, and
/// the notice afterwards repaints the item. The calls reach into other
/// processes and may wait on them, so this runs on a thread of its own and
/// never holds up the window; every step goes to refresh.log next to the cache.
fn refresh_shell_thumbnail(path: String, mut log: RefreshLog) {
    thread::spawn(move || {
        log.step("refresh thread running");
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            {
                let item: windows::core::Result<IShellItem> = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None);
                let cache: windows::core::Result<IThumbnailCache> =
                    CoCreateInstance(&LOCAL_THUMBNAIL_CACHE, None, CLSCTX_INPROC_SERVER);
                match (item, cache) {
                    (Ok(item), Ok(cache)) => {
                        for size in SHELL_THUMBNAIL_SIZES {
                            let mut bitmap: Option<ISharedBitmap> = None;
                            let result = cache.GetThumbnail(&item, size, WTS_FORCEEXTRACTION, Some(&mut bitmap), None, None);
                            log.step(&format!("{size}: {}", result.map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string())));
                        }
                    }
                    (item, cache) => log.step(&format!("setup failed: item {} cache {}", item.is_ok(), cache.is_ok())),
                }
            }
            SHChangeNotify(
                SHCNE_UPDATEITEM,
                SHCNF_FLAGS(SHCNF_PATHW.0 | SHCNF_FLUSHNOWAIT.0),
                Some(wide.as_ptr() as *const _),
                None,
            );
            log.step("notified");
            if init.is_ok() {
                CoUninitialize();
            }
        }
    });
}

/// refresh.log next to the cache: one block per saved thumbnail, each step
/// flushed as it happens, with the provider DLL's own lines in between (it
/// writes to the same file). Started over past 1 MB.
struct RefreshLog {
    file: Option<fs::File>,
    started: Instant,
}

impl RefreshLog {
    fn open(path: &str) -> Self {
        let file = thumbnail_dir().ok().and_then(|dir| {
            let log = dir.join("refresh.log");
            let oversized = fs::metadata(&log).map(|m| m.len() > 1 << 20).unwrap_or(false);
            fs::OpenOptions::new().append(!oversized).write(true).truncate(oversized).create(true).open(log).ok()
        });
        let mut log = RefreshLog { file, started: Instant::now() };
        log.step(&format!("--- {} pid {} {path}", unix_seconds(), std::process::id()));
        log
    }

    fn step(&mut self, text: &str) {
        if let Some(file) = &mut self.file {
            use std::io::Write;
            let _ = writeln!(file, "{:>6} ms  {text}", self.started.elapsed().as_millis());
            let _ = file.sync_data();
        }
    }
}

/// Seconds since the Unix epoch; enough to order log lines without a date crate.
fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Whether the file, as it is on disk right now, has a thumbnail.
#[tauri::command]
fn has_thumbnail(path: String) -> bool {
    match (thumbnail_dir(), thumbnail_key(&path)) {
        (Ok(dir), Ok(key)) => dir.join(format!("{key}.png")).is_file(),
        _ => false,
    }
}

/// Body: the thumbnail as PNG. Header: x-path (percent-encoded) of the model.
#[tauri::command]
fn save_thumbnail(request: Request<'_>) -> Result<(), String> {
    let path = percent_decode(header(&request, "x-path")?);
    let dir = thumbnail_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(format!("{}.png", thumbnail_key(&path)?));
    let bytes = raw_body(&request)?;
    fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut log = RefreshLog::open(&path);
    log.step(&format!("saved {} ({} bytes)", file.display(), bytes.len()));
    refresh_shell_thumbnail(path, log);
    Ok(())
}

// ---------------------------------------------------------------- install
//
// What install.ps1 does, from inside the app, for the current user (no admin):
// the page asks install_status() on start and, when the user agrees to the
// prompt, install_for_user() copies the exe and the thumbnail provider DLL to
// %LOCALAPPDATA%\Programs\ModelViewer, adds the app to "Open with" for .fbx
// and .obj and registers the provider. install.ps1 stays for scripted installs
// and for -Uninstall; the registry layout here must match it.

const INSTALL_DIR: &str = "Programs\\ModelViewer";
const PROVIDER_DLL: &str = "ModelViewerThumbnail.dll";
const PROVIDER_CLSID_TEXT: &str = "{9C1F2E7A-5B3D-4E86-A0F4-6D2B8C1E7F35}";
const THUMBNAIL_HANDLER: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";
const MODEL_EXTENSIONS: [&str; 2] = [".fbx", ".obj"];

#[derive(serde::Serialize)]
struct InstallStatus {
    install_dir: String,
    running_from_install: bool,
    files_current: bool,
    registered: bool,
    provider_found: bool,
}

fn install_dir() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(local).join(INSTALL_DIR))
}

/// The exe this process runs from and the provider DLL that ships next to it.
fn running_files() -> Result<(PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dll = exe.parent().ok_or("the app has no directory")?.join(PROVIDER_DLL);
    Ok((exe, dll))
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

fn same_content(a: &Path, b: &Path) -> bool {
    matches!((fs::read(a), fs::read(b)), (Ok(a), Ok(b)) if a == b)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn user_value(sub: &str, name: &str) -> Option<String> {
    let (sub, name) = (wide(sub), wide(name));
    let mut data = [0u16; 1024];
    let mut size = (data.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(data.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    (err.0 == 0).then(|| String::from_utf16_lossy(&data[..(size as usize / 2).saturating_sub(1)]))
}

fn user_dword(sub: &str, name: &str) -> Option<u32> {
    let (sub, name) = (wide(sub), wide(name));
    let mut data = 0u32;
    let mut size = 4u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut _ as *mut _),
            Some(&mut size),
        )
    };
    (err.0 == 0).then_some(data)
}

fn set_user_value(sub: &str, name: &str, kind: windows::Win32::System::Registry::REG_VALUE_TYPE, data: &[u8]) -> Result<(), String> {
    let (sub_w, name_w) = (wide(sub), wide(name));
    let mut key = HKEY::default();
    let err = unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(sub_w.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
    };
    if err.0 != 0 {
        return Err(format!("cannot create HKCU\\{sub}: error {}", err.0));
    }
    let err = unsafe { RegSetValueExW(key, PCWSTR(name_w.as_ptr()), None, kind, Some(data)) };
    let _ = unsafe { RegCloseKey(key) };
    if err.0 != 0 {
        return Err(format!("cannot write HKCU\\{sub}\\{name}: error {}", err.0));
    }
    Ok(())
}

fn set_sz(sub: &str, name: &str, value: &str) -> Result<(), String> {
    let bytes: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    set_user_value(sub, name, REG_SZ, &bytes)
}

fn set_dword(sub: &str, name: &str, value: u32) -> Result<(), String> {
    set_user_value(sub, name, REG_DWORD, &value.to_le_bytes())
}

fn open_with_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Whether the registry points at exactly these installed files.
fn registered_for(exe: &Path, dll: &Path) -> bool {
    let clsid = format!("Software\\Classes\\CLSID\\{PROVIDER_CLSID_TEXT}");
    user_value("Software\\Classes\\Applications\\ModelViewer.exe\\shell\\open\\command", "").as_deref()
        == Some(open_with_command(exe).as_str())
        && user_value(&format!("{clsid}\\InprocServer32"), "").as_deref() == Some(dll.display().to_string().as_str())
        && user_dword(&clsid, "DisableProcessIsolation") == Some(1)
        && MODEL_EXTENSIONS.iter().all(|ext| {
            user_value(&format!("Software\\Classes\\{ext}\\ShellEx\\{THUMBNAIL_HANDLER}"), "").as_deref()
                == Some(PROVIDER_CLSID_TEXT)
        })
}

fn register(exe: &Path, dll: &Path) -> Result<(), String> {
    let app = "Software\\Classes\\Applications\\ModelViewer.exe";
    set_sz(app, "FriendlyAppName", "Model Viewer")?;
    set_sz(&format!("{app}\\shell\\open\\command"), "", &open_with_command(exe))?;
    for ext in MODEL_EXTENSIONS {
        set_sz(&format!("{app}\\SupportedTypes"), ext, "")?;
    }
    // The provider runs inside Explorer: the shell's isolated thumbnail host
    // cannot create classes registered for the current user only.
    let clsid = format!("Software\\Classes\\CLSID\\{PROVIDER_CLSID_TEXT}");
    set_sz(&clsid, "", "Model Viewer Thumbnail Provider")?;
    set_dword(&clsid, "DisableProcessIsolation", 1)?;
    set_sz(&format!("{clsid}\\InprocServer32"), "", &dll.display().to_string())?;
    set_sz(&format!("{clsid}\\InprocServer32"), "ThreadingModel", "Apartment")?;
    for ext in MODEL_EXTENSIONS {
        set_sz(&format!("Software\\Classes\\{ext}\\ShellEx\\{THUMBNAIL_HANDLER}"), "", PROVIDER_CLSID_TEXT)?;
    }
    Ok(())
}

#[tauri::command]
fn install_status() -> Result<InstallStatus, String> {
    let dir = install_dir()?;
    let (exe, dll) = running_files()?;
    let (installed_exe, installed_dll) = (dir.join("ModelViewer.exe"), dir.join(PROVIDER_DLL));
    let running_from_install = same_file(&exe, &installed_exe);
    let files_current = running_from_install
        || (same_content(&exe, &installed_exe) && same_content(&dll, &installed_dll));
    Ok(InstallStatus {
        install_dir: dir.display().to_string(),
        running_from_install,
        files_current,
        registered: registered_for(&installed_exe, &installed_dll),
        provider_found: dll.is_file(),
    })
}

/// Copies the running exe and DLL into the install folder (unless running from
/// there), registers both, and tells Explorer. Returns the install folder.
#[tauri::command]
fn install_for_user() -> Result<String, String> {
    let dir = install_dir()?;
    let (exe, dll) = running_files()?;
    if !dll.is_file() {
        return Err(format!("{} is missing next to the app", dll.display()));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let installed_exe = dir.join("ModelViewer.exe");
    let installed_dll = dir.join(PROVIDER_DLL);
    if !same_file(&exe, &installed_exe) && !same_content(&exe, &installed_exe) {
        fs::copy(&exe, &installed_exe).map_err(|e| format!("cannot copy the app to {}: {e}", installed_exe.display()))?;
    }
    if !same_file(&dll, &installed_dll) && !same_content(&dll, &installed_dll) {
        // Explorer keeps the DLL loaded for a while; a loaded DLL cannot be
        // replaced in place but can be renamed aside. Parked copies that are
        // free again are removed next time.
        if installed_dll.exists() && fs::remove_file(&installed_dll).is_err() {
            let parked = dir.join(format!("ModelViewerThumbnail.{}.old", unix_seconds()));
            fs::rename(&installed_dll, &parked).map_err(|e| format!("cannot replace {}: {e}", installed_dll.display()))?;
        }
        fs::copy(&dll, &installed_dll).map_err(|e| format!("cannot copy the provider to {}: {e}", installed_dll.display()))?;
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "old") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    register(&installed_exe, &installed_dll)?;
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(dir.display().to_string())
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
            has_thumbnail,
            save_thumbnail,
            install_status,
            install_for_user,
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
