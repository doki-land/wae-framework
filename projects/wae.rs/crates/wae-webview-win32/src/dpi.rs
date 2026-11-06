//! Win32 DPI 感知、单位换算与诊断输出。
//!
//! 进程若未声明 DPI 感知，Windows 会把整个窗口位图整体拉伸（150% 下 1.5×），
//! 页面文字与 1 物理像素细节都会变糊。[`ensure_process_dpi_awareness`] 必须
//! 在任何窗口创建之前调用一次，之后窗口尺寸与 WebView2 边界统一按物理像素处理。

use std::{
    ffi::c_void,
    sync::atomic::{AtomicU8, Ordering},
};

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::{
        HiDpi::{
            AdjustWindowRectExForDpi, DPI_AWARENESS, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetAwarenessFromDpiAwarenessContext,
            GetDpiForSystem, GetDpiForWindow, GetThreadDpiAwarenessContext, SetProcessDpiAwarenessContext,
        },
        WindowsAndMessaging::{SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW, WINDOW_EX_STYLE, WINDOW_STYLE},
    },
};

/// 诊断开关环境变量：设为 `1`（或 `true`）时把 DPI 观测值打到 stderr。
pub(crate) const DIAG_ENV: &str = "WAE_WEBVIEW_DIAG";

const DIAG_UNKNOWN: u8 = 0;
const DIAG_OFF: u8 = 1;
const DIAG_ON: u8 = 2;

static DIAG: AtomicU8 = AtomicU8::new(DIAG_UNKNOWN);

/// 诊断是否开启（只读环境变量一次）。
pub(crate) fn diag_enabled() -> bool {
    let cached = DIAG.load(Ordering::Relaxed);
    if cached == DIAG_UNKNOWN {
        let on = std::env::var(DIAG_ENV).map(|value| value == "1" || value.eq_ignore_ascii_case("true")).unwrap_or(false);
        DIAG.store(if on { DIAG_ON } else { DIAG_OFF }, Ordering::Relaxed);
        return on;
    }
    cached == DIAG_ON
}

/// 输出一条以 `[wae-webview]` 开头的诊断行。
pub(crate) fn diag(message: impl AsRef<str>) {
    if diag_enabled() {
        eprintln!("[wae-webview] {}", message.as_ref());
    }
}

/// 进程 DPI 感知现状。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Awareness {
    /// `SetProcessDpiAwarenessContext(PerMonitorV2)` 是否成功。
    pub requested: bool,
    /// 调用后观察到的感知级别（0 unaware / 1 system / 2 per-monitor）。
    pub observed: DPI_AWARENESS,
}

impl Awareness {
    /// 稳定名称，用于诊断与验收。
    pub(crate) fn name(self) -> &'static str {
        match (self.requested, self.observed.0) {
            (true, _) => "per-monitor-v2",
            (false, 2) => "per-monitor-aware",
            (false, 1) => "system-aware",
            (false, 0) => "unaware",
            _ => "invalid",
        }
    }

    /// 是否会被 Windows 位图缩放（整片发虚）。
    pub(crate) fn is_bitmap_scaled(self) -> bool {
        !self.requested && self.observed.0 < 2
    }
}

/// 声明 Per-Monitor V2；**必须在创建任何窗口之前调用**。
///
/// 宿主进程（如 `node.exe`）自带清单可能已经固定了感知级别，此时调用失败会
/// 返回 `ERROR_ACCESS_DENIED`。函数不返回错误，只把真实结果交给调用方用于
/// 诊断：失败且仍为 unaware/system-aware 时窗口会被位图拉伸。
pub(crate) fn ensure_process_dpi_awareness() -> Awareness {
    let requested = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }.is_ok();
    let observed = unsafe { GetAwarenessFromDpiAwarenessContext(GetThreadDpiAwarenessContext()) };
    let report = Awareness { requested, observed };
    diag(format!("dpi.awareness set_per_monitor_v2={requested} observed={}", report.name()));
    if report.is_bitmap_scaled() {
        diag("dpi.awareness WARNING 进程无法升级为 Per-Monitor V2，窗口会被系统位图缩放");
    }
    report
}

/// 系统 DPI（未感知进程固定返回 96）。
pub(crate) fn system_dpi() -> u32 {
    unsafe { GetDpiForSystem() }
}

/// 窗口所在监视器的有效 DPI；取不到时退回系统 DPI。
pub(crate) fn window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { system_dpi() } else { dpi }
}

/// 逻辑像素（DIP，96 DPI 基准）→ 物理像素。
pub(crate) fn dip_to_physical(value: i32, dpi: u32) -> i32 {
    let numerator = i64::from(value) * i64::from(dpi);
    ((numerator + 48) / 96) as i32
}

/// 物理像素 → 逻辑像素（仅用于诊断输出）。
pub(crate) fn physical_to_dip(value: i32, dpi: u32) -> f64 {
    if dpi == 0 { f64::from(value) } else { f64::from(value) * 96.0 / f64::from(dpi) }
}

/// 由逻辑客户区尺寸推算带边框窗口的物理尺寸。
pub(crate) fn client_to_window_size(
    client_width_dip: i32,
    client_height_dip: i32,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    dpi: u32,
) -> (i32, i32) {
    let mut rect = RECT { left: 0, top: 0, right: dip_to_physical(client_width_dip, dpi), bottom: dip_to_physical(client_height_dip, dpi) };
    if unsafe { AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi) }.is_err() {
        return (rect.right, rect.bottom);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// 主监视器工作区（物理像素）。
pub(crate) fn primary_work_area() -> Option<RECT> {
    let mut rect = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut rect as *mut RECT as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
    };
    ok.ok().map(|()| rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dip_scales_match_windows_display_scales() {
        // 1100 × 720 DIP 在四档常见缩放下的物理尺寸。
        assert_eq!(dip_to_physical(1100, 96), 1100);
        assert_eq!(dip_to_physical(1100, 120), 1375);
        assert_eq!(dip_to_physical(1100, 144), 1650);
        assert_eq!(dip_to_physical(1100, 168), 1925);
        assert_eq!(dip_to_physical(1100, 175), 2005);
        assert_eq!(dip_to_physical(720, 96), 720);
        assert_eq!(dip_to_physical(720, 120), 900);
        assert_eq!(dip_to_physical(720, 144), 1080);
        assert_eq!(dip_to_physical(720, 168), 1260);
        assert_eq!(dip_to_physical(720, 175), 1313);
    }

    #[test]
    fn physical_to_dip_inverts_scaling() {
        assert!((physical_to_dip(1650, 144) - 1100.0).abs() < 0.001);
        assert!((physical_to_dip(1800, 144) - 1200.0).abs() < 0.001);
    }

    #[test]
    fn unaware_report_is_bitmap_scaled() {
        let unaware = Awareness { requested: false, observed: DPI_AWARENESS(0) };
        assert_eq!(unaware.name(), "unaware");
        assert!(unaware.is_bitmap_scaled());

        let v2 = Awareness { requested: true, observed: DPI_AWARENESS(2) };
        assert_eq!(v2.name(), "per-monitor-v2");
        assert!(!v2.is_bitmap_scaled());
    }
}
