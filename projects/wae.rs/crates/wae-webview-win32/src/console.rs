//! 控制台窗口收尾。
//!
//! `node.exe` 是 CUI 子系统程序。启动它的进程没有控制台时（资源管理器、
//! 隐藏启动器、不分配控制台的自动化宿主），Windows 会为它新建一个**可见**的
//! `ConsoleWindowClass` 窗口，标题默认取 exe 全路径；GUI 宿主随后在同一进程里
//! 创建产品窗口，于是用户旁边一直挂着一个黑色空控制台。
//!
//! 从终端启动时控制台属于用户自己的终端（同一控制台上至少还挂着 shell），
//! 既不能隐藏也不能另开窗口。区分依据是控制台上挂着的进程数。

use windows::Win32::{
    Foundation::HWND,
    System::Console::{GetConsoleProcessList, GetConsoleWindow},
    UI::WindowsAndMessaging::{IsWindowVisible, SW_HIDE, ShowWindow},
};

/// 控制台观测与处理结果，用于结构化诊断。
pub(crate) struct ConsoleReport {
    /// 进程是否挂着控制台窗口。
    pub(crate) window: bool,
    /// 同一控制台上挂着的进程数（0 表示查询失败）。
    pub(crate) processes: u32,
    /// 是否已把窗口隐藏。
    pub(crate) hidden: bool,
}

/// 若控制台只挂着本进程，隐藏它的窗口。
///
/// 只隐藏、不 `FreeConsole`：`FreeConsole` 会让宿主的 stdout/stderr 句柄失效，
/// 之后 `process.stderr.write` 可能以未处理的错误事件让 GUI 进程崩溃。隐藏后
/// stdio 仍然可用，只是没人看见；需要留痕的诊断走文件日志。
pub(crate) fn hide_orphan_console() -> ConsoleReport {
    let hwnd = unsafe { GetConsoleWindow() };
    if hwnd == HWND::default() {
        return ConsoleReport { window: false, processes: 0, hidden: false };
    }

    let mut list = [0u32; 32];
    let processes = unsafe { GetConsoleProcessList(&mut list) };
    // 查询失败（0）时不做处理：宁可留下窗口，也不冒险隐藏用户的终端。
    if processes != 1 {
        return ConsoleReport { window: true, processes, hidden: false };
    }

    let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    let hidden = !unsafe { IsWindowVisible(hwnd) }.as_bool();
    ConsoleReport { window: true, processes, hidden }
}
