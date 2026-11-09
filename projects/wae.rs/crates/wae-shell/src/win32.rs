//! Windows 启动器：`ShellExecuteW("open")`。

use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};

use windows::{
    Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    core::PCWSTR,
};

use crate::ShellError;

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(std::iter::once(0)).collect()
}

pub fn open_path(path: &Path) -> Result<(), ShellError> {
    if !path.is_file() {
        return Err(ShellError::Missing { path: path.display().to_string() });
    }
    let verb = wide("open");
    let file = wide(&path.display().to_string());
    let directory = wide(&path.parent().map(|parent| parent.display().to_string()).unwrap_or_default());
    let instance =
        unsafe { ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(file.as_ptr()), PCWSTR::null(), PCWSTR(directory.as_ptr()), SW_SHOWNORMAL) };
    let status = instance.0 as isize;
    if status > 32 {
        return Ok(());
    }
    Err(ShellError::LaunchFailed { launcher: "ShellExecuteW", message: format!("ShellExecuteW status {status}") })
}
