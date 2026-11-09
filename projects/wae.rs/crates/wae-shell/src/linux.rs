//! Linux 启动器：`xdg-open`。

use std::{path::Path, process::Command};

use crate::ShellError;

pub fn open_path(path: &Path) -> Result<(), ShellError> {
    if !path.is_file() {
        return Err(ShellError::Missing { path: path.display().to_string() });
    }
    match Command::new("xdg-open").arg(path).spawn() {
        Ok(_child) => Ok(()),
        Err(error) => Err(ShellError::LaunchFailed { launcher: "xdg-open", message: error.to_string() }),
    }
}
