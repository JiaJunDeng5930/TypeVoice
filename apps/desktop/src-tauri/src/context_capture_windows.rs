#![cfg(windows)]

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, RGBQUAD,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow,
};

pub struct WindowInfo {
    pub title: Option<String>,
    pub process_image: Option<String>,
}

pub struct ScreenshotRaw {
    pub png_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone)]
pub struct WindowsContext {
    tracker: ForegroundTracker,
}

impl WindowsContext {
    pub fn new() -> Self {
        Self {
            tracker: ForegroundTracker::new(),
        }
    }

    pub fn warmup_best_effort(&self) {
        self.tracker.ensure_started();
    }

    pub fn last_external_window_info_best_effort(&self) -> Option<WindowInfo> {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        let hwnd = snap.hwnd? as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return None;
        }
        Some(WindowInfo {
            title: get_window_title_best_effort(hwnd),
            process_image: snap
                .process_image
                .or_else(|| get_process_image_best_effort(snap.pid)),
        })
    }

    pub fn capture_last_external_window_png_best_effort(
        &self,
        max_side: u32,
    ) -> Option<ScreenshotRaw> {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        let hwnd = snap.hwnd? as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return None;
        }
        capture_window_png_best_effort(hwnd, max_side)
    }

    pub fn read_clipboard_text_best_effort(&self) -> Option<String> {
        read_clipboard_text_best_effort()
    }
}

#[derive(Debug, Clone)]
struct ExternalSnapshot {
    // HWND is a raw pointer type and is not Send/Sync. Store it as an integer so that
    // the tracker can live inside Tauri managed state (which requires Send + Sync).
    hwnd: Option<isize>,
    pid: u32,
    process_image: Option<String>,
}

#[derive(Clone)]
struct ForegroundTracker {
    started: Arc<AtomicBool>,
    last_external: Arc<Mutex<ExternalSnapshot>>,
}

impl ForegroundTracker {
    fn new() -> Self {
        Self {
            started: Arc::new(AtomicBool::new(false)),
            last_external: Arc::new(Mutex::new(ExternalSnapshot {
                hwnd: None,
                pid: 0,
                process_image: None,
            })),
        }
    }

    fn ensure_started(&self) {
        if self.started.load(Ordering::SeqCst) {
            return;
        }
        if self
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let last_external = self.last_external.clone();
        let this_pid = std::process::id();
        std::thread::Builder::new()
            .name("foreground_tracker".to_string())
            .spawn(move || loop {
                let hwnd = unsafe { GetForegroundWindow() };
                if !hwnd.is_null() {
                    let mut pid: u32 = 0;
                    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                    if pid != 0 && pid != this_pid {
                        let img = get_process_image_best_effort(pid);
                        let mut g = last_external.lock().unwrap();
                        g.hwnd = Some(hwnd as isize);
                        g.pid = pid;
                        g.process_image = img;
                    }
                }
                std::thread::sleep(Duration::from_millis(80));
            })
            .ok();
    }

    fn last_external_snapshot(&self) -> ExternalSnapshot {
        self.last_external.lock().unwrap().clone()
    }
}

fn get_window_title_best_effort(hwnd: HWND) -> Option<String> {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return None;
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(String::from_utf16_lossy(&buf).trim().to_string())
}

fn get_process_image_best_effort(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 260];
        let mut size: u32 = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size);
        let _ = CloseHandle(h);
        if ok == 0 || size == 0 {
            return None;
        }
        buf.truncate(size as usize);
        Some(String::from_utf16_lossy(&buf).trim().to_string())
    }
}

fn capture_window_png_best_effort(hwnd: HWND, max_side: u32) -> Option<ScreenshotRaw> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    if ok == 0 {
        return None;
    }
    let w = (rect.right - rect.left).max(0) as u32;
    let h = (rect.bottom - rect.top).max(0) as u32;
    if w == 0 || h == 0 {
        return None;
    }

    // Create a memory DC + bitmap and use PrintWindow.
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return None;
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return None;
        }

        let bmp = CreateCompatibleBitmap(screen_dc, w as i32, h as i32);
        if bmp.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return None;
        }

        let old = SelectObject(mem_dc, bmp as _);
        let pw_ok = PrintWindow(hwnd, mem_dc, 0);
        ReleaseDC(std::ptr::null_mut(), screen_dc);

        if pw_ok == 0 {
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(bmp as _);
            let _ = DeleteDC(mem_dc);
            return None;
        }

        let (out_w, out_h) = clamp_size(w, h, max_side);
        let mut rgba = vec![0u8; (out_w as usize) * (out_h as usize) * 4];

        // Read raw BGRA pixels first, then resize/convert in one pass.
        let mut src_bgra = vec![0u8; (w as usize) * (h as usize) * 4];
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                // Negative height requests a top-down DIB (no vertical flip needed).
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB as u32,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbReserved: 0,
            }; 1],
        };

        let got = GetDIBits(
            mem_dc,
            bmp,
            0,
            h as u32,
            src_bgra.as_mut_ptr() as *mut c_void,
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = SelectObject(mem_dc, old);
        let _ = DeleteObject(bmp as _);
        let _ = DeleteDC(mem_dc);
        if got == 0 {
            return None;
        }

        resize_convert_bgra_to_rgba(&src_bgra, w, h, &mut rgba, out_w, out_h);
        let png_bytes = encode_png_rgba(&rgba, out_w, out_h)?;
        Some(ScreenshotRaw {
            png_bytes,
            width: out_w,
            height: out_h,
        })
    }
}

fn clamp_size(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    if max_side == 0 {
        return (w, h);
    }
    let m = w.max(h);
    if m <= max_side {
        return (w, h);
    }
    let scale = max_side as f64 / (m as f64);
    let nw = ((w as f64) * scale).round().max(1.0) as u32;
    let nh = ((h as f64) * scale).round().max(1.0) as u32;
    (nw, nh)
}

fn resize_convert_bgra_to_rgba(
    src_bgra: &[u8],
    src_w: u32,
    src_h: u32,
    dst_rgba: &mut [u8],
    dst_w: u32,
    dst_h: u32,
) {
    // Nearest-neighbor resize + BGRA -> RGBA conversion.
    for y in 0..dst_h {
        let sy = (y as u64 * src_h as u64 / dst_h as u64) as u32;
        for x in 0..dst_w {
            let sx = (x as u64 * src_w as u64 / dst_w as u64) as u32;
            let sidx = ((sy * src_w + sx) as usize) * 4;
            let didx = ((y * dst_w + x) as usize) * 4;
            let b = src_bgra.get(sidx).copied().unwrap_or(0);
            let g = src_bgra.get(sidx + 1).copied().unwrap_or(0);
            let r = src_bgra.get(sidx + 2).copied().unwrap_or(0);
            let a = src_bgra.get(sidx + 3).copied().unwrap_or(255);
            dst_rgba[didx] = r;
            dst_rgba[didx + 1] = g;
            dst_rgba[didx + 2] = b;
            dst_rgba[didx + 3] = a;
        }
    }
}

fn encode_png_rgba(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

fn read_clipboard_text_best_effort() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 {
            return None;
        }
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return None;
        }

        // Find NUL terminator.
        let mut len = 0usize;
        loop {
            let v = *ptr.add(len);
            if v == 0 {
                break;
            }
            len += 1;
            // guard against absurd clipboard sizes
            if len > 200_000 {
                break;
            }
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let s = String::from_utf16_lossy(slice).trim().to_string();
        let _ = GlobalUnlock(handle);
        let _ = CloseClipboard();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}
