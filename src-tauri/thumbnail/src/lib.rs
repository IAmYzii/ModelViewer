// Explorer thumbnails for .fbx and .obj files: an in-process COM server
// (IThumbnailProvider + IInitializeWithStream) that Explorer, and any file
// manager built on the shell, loads to get a picture of a model.
//
// It renders nothing itself. Model Viewer saves a PNG of every model it opens
// into %LOCALAPPDATA%\ModelViewer\thumbnails\<key>.png (see `save_thumbnail`
// in ../src/main.rs); <key> is the file's content hash, computed the same way
// here, so a model keeps its thumbnail when moved or copied and loses it when
// re-exported. A file that was never opened in the viewer has no entry and
// Explorer falls back to the plain icon.
//
// Explorer loads it in-process (install.ps1 sets DisableProcessIsolation: the
// shell's isolated thumbnail host cannot create per-user registered classes),
// so nothing in here may panic: the release profile aborts on panic and would
// take Explorer down with it. Every failure is an HRESULT instead.
#![allow(non_snake_case)]

use std::{
    cell::RefCell,
    ffi::c_void,
    fs::File,
    io::BufReader,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use sha2::{Digest, Sha256};
use windows::{
    core::{implement, Error, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT},
    Win32::{
        Foundation::{CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_POINTER, E_UNEXPECTED, S_FALSE, S_OK},
        Graphics::Gdi::{CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP},
        System::Com::{IClassFactory, IClassFactory_Impl, IStream},
        UI::Shell::{
            IThumbnailProvider, IThumbnailProvider_Impl,
            PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl},
            WTSAT_ARGB, WTS_ALPHATYPE,
        },
    },
};

/// The provider's CLSID; install.ps1 registers it under HKCU.
pub const CLSID: GUID = GUID::from_u128(0x9c1f2e7a_5b3d_4e86_a0f4_6d2b8c1e7f35);

/// Live COM objects. While it is zero the host may unload the DLL, which is
/// how a newer copy gets picked up without restarting Explorer.
static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Lease;

impl Lease {
    fn take() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Lease
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------- cache
fn fail<E>(_: E) -> Error {
    E_FAIL.into()
}

fn cache_dir() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or_else(|| Error::from(E_FAIL))?;
    Ok(PathBuf::from(local).join("ModelViewer").join("thumbnails"))
}

fn cache_path(key: &str) -> Result<PathBuf> {
    Ok(cache_dir()?.join(format!("{key}.png")))
}

/// One line per request in refresh.log next to the cache, the same file the
/// viewer writes its side into. The log is started over past 1 MB.
fn note(text: &str) {
    let Ok(dir) = cache_dir() else { return };
    let path = dir.join("refresh.log");
    let oversized = std::fs::metadata(&path).map(|m| m.len() > 1 << 20).unwrap_or(false);
    let file = std::fs::OpenOptions::new().append(!oversized).write(true).truncate(oversized).create(true).open(path);
    if let Ok(mut file) = file {
        use std::io::Write;
        let _ = writeln!(file, "           provider in pid {}: {text}", std::process::id());
    }
}

/// SHA-256 of the file's bytes, first 16 bytes as hex. Must match
/// `thumbnail_key` in ../src/main.rs.
fn content_key(stream: &IStream) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let mut read = 0u32;
        let hr = unsafe { stream.Read(buf.as_mut_ptr() as *mut c_void, buf.len() as u32, Some(&mut read)) };
        if hr.is_err() {
            return Err(hr.into());
        }
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read as usize]);
    }
    Ok(hasher.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect())
}

// ---------------------------------------------------------------- image
/// Straight (not premultiplied) RGBA, top row first.
struct Image {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

fn load_png(path: &PathBuf) -> Result<Image> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path).map_err(fail)?));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(fail)?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or_else(|| Error::from(E_FAIL))?];
    let info = reader.next_frame(&mut buf).map_err(fail)?;
    let (width, height) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return Err(E_FAIL.into()),
    };
    let bytes = buf.get(..width * height * channels).ok_or_else(|| Error::from(E_FAIL))?;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for px in bytes.chunks_exact(channels) {
        match channels {
            4 => rgba.extend_from_slice(px),
            3 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
            2 => rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            _ => rgba.extend_from_slice(&[px[0], px[0], px[0], 255]),
        }
    }
    Ok(Image { width, height, rgba })
}

/// GDI wants alpha premultiplied; averaging premultiplied pixels also keeps
/// transparent edges from darkening when the image is shrunk.
fn premultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in 0..3 {
            px[c] = ((px[c] as u32 * a + 127) / 255) as u8;
        }
    }
}

/// Shrink so the longer side is at most `max`, averaging the source pixels
/// each output pixel covers. Images that already fit are returned as they are.
fn fit(image: Image, max: usize) -> Image {
    let longest = image.width.max(image.height);
    if max == 0 || longest <= max {
        return image;
    }
    let width = (image.width * max / longest).max(1);
    let height = (image.height * max / longest).max(1);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let y0 = y * image.height / height;
        let y1 = ((y + 1) * image.height / height).max(y0 + 1);
        for x in 0..width {
            let x0 = x * image.width / width;
            let x1 = ((x + 1) * image.width / width).max(x0 + 1);
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * image.width + sx) * 4;
                    for c in 0..4 {
                        sum[c] += image.rgba[i + c] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            for c in 0..4 {
                rgba.push((sum[c] / n) as u8);
            }
        }
    }
    Image { width, height, rgba }
}

/// A top-down 32-bit DIB holding the (premultiplied) image as BGRA.
fn dib_from(image: &Image) -> Result<HBITMAP> {
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: image.width as i32,
            biHeight: -(image.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bitmap = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)? };
    if bits.is_null() {
        return Err(E_FAIL.into());
    }
    let out = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, image.width * image.height * 4) };
    for (dst, src) in out.chunks_exact_mut(4).zip(image.rgba.chunks_exact(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
    Ok(bitmap)
}

// ---------------------------------------------------------------- COM objects
#[implement(IThumbnailProvider, IInitializeWithStream)]
struct Provider {
    stream: RefCell<Option<IStream>>,
    _lease: Lease,
}

impl IInitializeWithStream_Impl for Provider_Impl {
    fn Initialize(&self, pstream: Ref<IStream>, _grfmode: u32) -> Result<()> {
        *self.stream.borrow_mut() = Some(pstream.ok()?.clone());
        Ok(())
    }
}

impl IThumbnailProvider_Impl for Provider_Impl {
    fn GetThumbnail(&self, cx: u32, phbmp: *mut HBITMAP, pdwalpha: *mut WTS_ALPHATYPE) -> Result<()> {
        if phbmp.is_null() || pdwalpha.is_null() {
            return Err(E_POINTER.into());
        }
        let stream = self.stream.borrow().clone().ok_or_else(|| Error::from(E_UNEXPECTED))?;
        let key = content_key(&stream).map_err(|e| {
            note(&format!("cannot read the stream: {e}"));
            e
        })?;
        let path = cache_path(&key)?;
        note(&format!("{cx} px for {key}: {}", if path.is_file() { "cached" } else { "no thumbnail yet" }));
        let mut image = load_png(&path)?;
        premultiply(&mut image.rgba);
        let image = fit(image, cx as usize);
        let bitmap = dib_from(&image)?;
        unsafe {
            *phbmp = bitmap;
            *pdwalpha = WTSAT_ARGB;
        }
        Ok(())
    }
}

#[implement(IClassFactory)]
struct Factory {
    _lease: Lease,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, punkouter: Ref<IUnknown>, riid: *const GUID, ppvobject: *mut *mut c_void) -> Result<()> {
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider { stream: RefCell::new(None), _lease: Lease::take() }.into();
        unsafe { provider.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- DLL exports
#[no_mangle]
pub extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    if unsafe { *rclsid } != CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IUnknown = Factory { _lease: Lease::take() }.into();
    unsafe { factory.query(riid, ppv) }
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
