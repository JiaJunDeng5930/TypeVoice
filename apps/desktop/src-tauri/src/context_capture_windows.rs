#![cfg(windows)]

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde::Serialize;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HWND, RECT};
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

#[derive(Debug, Clone)]
pub struct ForegroundNowCapture {
    pub window: WindowInfo,
    pub screenshot: ScreenshotRaw,
    pub pid: u32,
    pub hwnd: isize,
}

#[derive(Debug, Clone)]
pub struct ForegroundNowCaptureResult {
    pub capture: Option<ForegroundNowCapture>,
    pub error: Option<ScreenshotDiagError>,
}

#[derive(Clone)]
pub struct ScreenshotRaw {
    pub png_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for ScreenshotRaw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid dumping raw pixels into logs accidentally.
        f.debug_struct("ScreenshotRaw")
            .field("png_bytes_len", &self.png_bytes.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotDiagError {
    pub step: String,
    pub api: String,
    pub api_ret: String,
    pub last_error: u32,
    pub note: Option<String>,
    pub window_w: u32,
    pub window_h: u32,
    pub max_side: u32,
}

#[derive(Debug, Clone)]
pub struct ScreenshotDiagResult {
    pub raw: Option<ScreenshotRaw>,
    pub error: Option<ScreenshotDiagError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipboardDiag {
    pub status: String, // ok|skipped|err
    pub step: Option<String>,
    pub last_error: Option<u32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClipboardRead {
    pub text: Option<String>,
    pub diag: ClipboardDiag,
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
        self.capture_last_external_window_png_diag_best_effort(max_side)
            .raw
    }

    pub fn capture_last_external_window_png_diag_best_effort(
        &self,
        max_side: u32,
    ) -> ScreenshotDiagResult {
        self.tracker.ensure_started();
        let snap = self.tracker.last_external_snapshot();
        let Some(hwnd_i) = snap.hwnd else {
            return ScreenshotDiagResult {
                raw: None,
                error: None,
            };
        };
        let hwnd = hwnd_i as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return ScreenshotDiagResult {
                raw: None,
                error: None,
            };
        }
        match capture_window_png_diagnose(hwnd, max_side) {
            Ok(raw) => ScreenshotDiagResult {
                raw: Some(raw),
                error: None,
            },
            Err(e) => ScreenshotDiagResult {
                raw: None,
                error: Some(e),
            },
        }
    }

    pub fn read_clipboard_text_best_effort(&self) -> Option<String> {
        self.read_clipboard_text_diag_best_effort().text
    }

    pub fn read_clipboard_text_diag_best_effort(&self) -> ClipboardRead {
        match read_clipboard_text_diagnose() {
            Ok(Some(s)) => ClipboardRead {
                text: Some(s),
                diag: ClipboardDiag {
                    status: "ok".to_string(),
                    step: None,
                    last_error: None,
                    note: None,
                },
            },
            Ok(None) => ClipboardRead {
                text: None,
                diag: ClipboardDiag {
                    status: "skipped".to_string(),
                    step: None,
                    last_error: None,
                    note: Some("empty_or_unavailable".to_string()),
                },
            },
            Err(e) => ClipboardRead {
                text: None,
                diag: ClipboardDiag {
                    status: "err".to_string(),
                    step: Some(e.step),
                    last_error: Some(e.last_error),
                    note: Some(e.note),
                },
            },
        }
    }

    pub fn capture_foreground_window_now_diag_best_effort(
        &self,
        max_side: u32,
    ) -> ForegroundNowCaptureResult {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return ForegroundNowCaptureResult {
                capture: None,
                error: Some(ScreenshotDiagError {
                    step: "foreground_window".to_string(),
                    api: "GetForegroundWindow".to_string(),
                    api_ret: "NULL".to_string(),
                    last_error: last_error_u32(),
                    note: Some("no foreground window".to_string()),
                    window_w: 0,
                    window_h: 0,
                    max_side,
                }),
            };
        }
        if unsafe { IsWindow(hwnd) } == 0 {
            return ForegroundNowCaptureResult {
                capture: None,
                error: Some(ScreenshotDiagError {
                    step: "is_window".to_string(),
                    api: "IsWindow".to_string(),
                    api_ret: "0".to_string(),
                    last_error: last_error_u32(),
                    note: Some("foreground hwnd is invalid".to_string()),
                    window_w: 0,
                    window_h: 0,
                    max_side,
                }),
            };
        }

        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return ForegroundNowCaptureResult {
                capture: None,
                error: Some(ScreenshotDiagError {
                    step: "foreground_pid".to_string(),
                    api: "GetWindowThreadProcessId".to_string(),
                    api_ret: "pid=0".to_string(),
                    last_error: last_error_u32(),
                    note: Some("foreground pid is zero".to_string()),
                    window_w: 0,
                    window_h: 0,
                    max_side,
                }),
            };
        }

        let info = WindowInfo {
            title: get_window_title_best_effort(hwnd),
            process_image: get_process_image_best_effort(pid),
        };
        match capture_window_png_diagnose(hwnd, max_side) {
            Ok(raw) => ForegroundNowCaptureResult {
                capture: Some(ForegroundNowCapture {
                    window: info,
                    screenshot: raw,
                    pid,
                    hwnd: hwnd as isize,
                }),
                error: None,
            },
            Err(e) => ForegroundNowCaptureResult {
                capture: None,
                error: Some(e),
            },
        }
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

fn last_error_u32() -> u32 {
    unsafe { GetLastError() }
}

fn screenshot_err(
    step: &str,
    api: &str,
    api_ret: String,
    note: Option<String>,
    window_w: u32,
    window_h: u32,
    max_side: u32,
) -> ScreenshotDiagError {
    ScreenshotDiagError {
        step: step.to_string(),
        api: api.to_string(),
        api_ret,
        last_error: last_error_u32(),
        note,
        window_w,
        window_h,
        max_side,
    }
}

fn capture_window_png_diagnose(
    hwnd: HWND,
    max_side: u32,
) -> Result<ScreenshotRaw, ScreenshotDiagError> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    if ok == 0 {
        return Err(screenshot_err(
            "get_window_rect",
            "GetWindowRect",
            "0".to_string(),
            None,
            0,
            0,
            max_side,
        ));
    }
    let w = (rect.right - rect.left).max(0) as u32;
    let h = (rect.bottom - rect.top).max(0) as u32;
    if w == 0 || h == 0 {
        // Not a WinAPI failure; still record as a diagnosable step.
        return Err(ScreenshotDiagError {
            step: "window_size".to_string(),
            api: "GetWindowRect".to_string(),
            api_ret: format!("w={w} h={h}"),
            last_error: 0,
            note: Some("window has zero size".to_string()),
            window_w: w,
            window_h: h,
            max_side,
        });
    }

    // Create a memory DC + bitmap and use PrintWindow.
    unsafe {
        let screen_dc = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return Err(screenshot_err(
                "get_dc",
                "GetDC",
                "NULL".to_string(),
                None,
                w,
                h,
                max_side,
            ));
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return Err(screenshot_err(
                "create_compatible_dc",
                "CreateCompatibleDC",
                "NULL".to_string(),
                None,
                w,
                h,
                max_side,
            ));
        }

        let bmp = CreateCompatibleBitmap(screen_dc, w as i32, h as i32);
        if bmp.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(std::ptr::null_mut(), screen_dc);
            return Err(screenshot_err(
                "create_compatible_bitmap",
                "CreateCompatibleBitmap",
                "NULL".to_string(),
                None,
                w,
                h,
                max_side,
            ));
        }

        let old = SelectObject(mem_dc, bmp as _);
        let hgdi_error = (-1isize) as *mut c_void;
        if old.is_null() || old == hgdi_error {
            let _ = DeleteObject(bmp as _);
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(std::ptr::null_mut(), screen_dc);
            return Err(screenshot_err(
                "select_object",
                "SelectObject",
                format!("{old:?}"),
                Some("SelectObject failed".to_string()),
                w,
                h,
                max_side,
            ));
        }
        let pw_ok = PrintWindow(hwnd, mem_dc, 0);
        ReleaseDC(std::ptr::null_mut(), screen_dc);

        if pw_ok == 0 {
            let _ = SelectObject(mem_dc, old);
            let _ = DeleteObject(bmp as _);
            let _ = DeleteDC(mem_dc);
            return Err(screenshot_err(
                "print_window",
                "PrintWindow",
                "0".to_string(),
                None,
                w,
                h,
                max_side,
            ));
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
            return Err(screenshot_err(
                "get_dibits",
                "GetDIBits",
                "0".to_string(),
                None,
                w,
                h,
                max_side,
            ));
        }

        if is_effectively_black_bgra(&src_bgra) {
            return Err(ScreenshotDiagError {
                step: "validate_pixels".to_string(),
                api: "pixel_check".to_string(),
                api_ret: "all_black".to_string(),
                last_error: 0,
                note: Some("captured frame is effectively black".to_string()),
                window_w: w,
                window_h: h,
                max_side,
            });
        }

        resize_convert_bgra_to_rgba(&src_bgra, w, h, &mut rgba, out_w, out_h);
        let png_bytes =
            encode_png_rgba(&rgba, out_w, out_h).ok_or_else(|| ScreenshotDiagError {
                step: "encode_png".to_string(),
                api: "png::Encoder".to_string(),
                api_ret: "None".to_string(),
                last_error: 0,
                note: Some("encode_png_rgba returned None".to_string()),
                window_w: w,
                window_h: h,
                max_side,
            })?;
        Ok(ScreenshotRaw {
            png_bytes,
            width: out_w,
            height: out_h,
        })
    }
}

fn is_effectively_black_bgra(src_bgra: &[u8]) -> bool {
    if src_bgra.len() < 4 {
        return true;
    }
    let px_count = src_bgra.len() / 4;
    let stride = (px_count / 4096).max(1);
    let mut sampled = 0usize;
    let mut bright = 0usize;
    let mut i = 0usize;
    while i < px_count {
        let idx = i * 4;
        let b = src_bgra[idx] as f32;
        let g = src_bgra[idx + 1] as f32;
        let r = src_bgra[idx + 2] as f32;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        sampled += 1;
        if y > 20.0 {
            bright += 1;
        }
        i += stride;
    }
    bright * 1000 <= sampled
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
    if src_w == dst_w && src_h == dst_h {
        // Fast path: just convert BGRA -> RGBA.
        for y in 0..dst_h {
            for x in 0..dst_w {
                let sidx = ((y * src_w + x) as usize) * 4;
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
        return;
    }

    // Bilinear resize + BGRA -> RGBA conversion.
    // This improves readability for downscaled UI screenshots compared to nearest-neighbor.
    let src_w_f = (src_w as f32).max(1.0);
    let src_h_f = (src_h as f32).max(1.0);
    let dst_w_f = (dst_w as f32).max(1.0);
    let dst_h_f = (dst_h as f32).max(1.0);

    for y in 0..dst_h {
        // Center-sampling mapping (reduces aliasing compared to edge mapping).
        let fy = ((y as f32) + 0.5) * (src_h_f / dst_h_f) - 0.5;
        let fy = fy.clamp(0.0, src_h_f - 1.0);
        let y0 = fy.floor() as u32;
        let y1 = (y0 + 1).min(src_h.saturating_sub(1));
        let wy = fy - (y0 as f32);

        for x in 0..dst_w {
            let fx = ((x as f32) + 0.5) * (src_w_f / dst_w_f) - 0.5;
            let fx = fx.clamp(0.0, src_w_f - 1.0);
            let x0 = fx.floor() as u32;
            let x1 = (x0 + 1).min(src_w.saturating_sub(1));
            let wx = fx - (x0 as f32);

            let p00 = ((y0 * src_w + x0) as usize) * 4;
            let p10 = ((y0 * src_w + x1) as usize) * 4;
            let p01 = ((y1 * src_w + x0) as usize) * 4;
            let p11 = ((y1 * src_w + x1) as usize) * 4;

            let b00 = src_bgra.get(p00).copied().unwrap_or(0) as f32;
            let g00 = src_bgra.get(p00 + 1).copied().unwrap_or(0) as f32;
            let r00 = src_bgra.get(p00 + 2).copied().unwrap_or(0) as f32;
            let a00 = src_bgra.get(p00 + 3).copied().unwrap_or(255) as f32;

            let b10 = src_bgra.get(p10).copied().unwrap_or(0) as f32;
            let g10 = src_bgra.get(p10 + 1).copied().unwrap_or(0) as f32;
            let r10 = src_bgra.get(p10 + 2).copied().unwrap_or(0) as f32;
            let a10 = src_bgra.get(p10 + 3).copied().unwrap_or(255) as f32;

            let b01 = src_bgra.get(p01).copied().unwrap_or(0) as f32;
            let g01 = src_bgra.get(p01 + 1).copied().unwrap_or(0) as f32;
            let r01 = src_bgra.get(p01 + 2).copied().unwrap_or(0) as f32;
            let a01 = src_bgra.get(p01 + 3).copied().unwrap_or(255) as f32;

            let b11 = src_bgra.get(p11).copied().unwrap_or(0) as f32;
            let g11 = src_bgra.get(p11 + 1).copied().unwrap_or(0) as f32;
            let r11 = src_bgra.get(p11 + 2).copied().unwrap_or(0) as f32;
            let a11 = src_bgra.get(p11 + 3).copied().unwrap_or(255) as f32;

            let b0 = b00 * (1.0 - wx) + b10 * wx;
            let g0 = g00 * (1.0 - wx) + g10 * wx;
            let r0 = r00 * (1.0 - wx) + r10 * wx;
            let a0 = a00 * (1.0 - wx) + a10 * wx;

            let b1 = b01 * (1.0 - wx) + b11 * wx;
            let g1 = g01 * (1.0 - wx) + g11 * wx;
            let r1 = r01 * (1.0 - wx) + r11 * wx;
            let a1 = a01 * (1.0 - wx) + a11 * wx;

            let b = b0 * (1.0 - wy) + b1 * wy;
            let g = g0 * (1.0 - wy) + g1 * wy;
            let r = r0 * (1.0 - wy) + r1 * wy;
            let a = a0 * (1.0 - wy) + a1 * wy;

            let didx = ((y * dst_w + x) as usize) * 4;
            dst_rgba[didx] = r.round().clamp(0.0, 255.0) as u8;
            dst_rgba[didx + 1] = g.round().clamp(0.0, 255.0) as u8;
            dst_rgba[didx + 2] = b.round().clamp(0.0, 255.0) as u8;
            dst_rgba[didx + 3] = a.round().clamp(0.0, 255.0) as u8;
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

#[derive(Debug, Clone)]
struct ClipboardDiagError {
    step: String,
    last_error: u32,
    note: String,
}

fn read_clipboard_text_diagnose() -> Result<Option<String>, ClipboardDiagError> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 {
            return Ok(None);
        }
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return Err(ClipboardDiagError {
                step: "open_clipboard".to_string(),
                last_error: GetLastError(),
                note: "OpenClipboard failed".to_string(),
            });
        }
        let handle = GetClipboardData(CF_UNICODETEXT as u32);
        if handle.is_null() {
            let _ = CloseClipboard();
            return Err(ClipboardDiagError {
                step: "get_clipboard_data".to_string(),
                last_error: GetLastError(),
                note: "GetClipboardData returned NULL".to_string(),
            });
        }
        let ptr = GlobalLock(handle) as *const u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return Err(ClipboardDiagError {
                step: "global_lock".to_string(),
                last_error: GetLastError(),
                note: "GlobalLock returned NULL".to_string(),
            });
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
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }
}
