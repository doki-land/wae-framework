//! Windows 进程内回读校验：经系统 `msi.dll` 的 SQL 视图逐表读回，
//! 不 spawn 任何 exe。非 Windows 目标编译为结构化 `Unsupported`。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{InstallerError, Result};

/// 回读报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    /// 被校验产物。
    pub path: std::path::PathBuf,
    /// `_Tables` 枚举出的表名。
    pub tables: Vec<String>,
    /// 每表行数。
    pub rows: Vec<(String, usize)>,
}

/// 打开产物并逐表回读；任何表查询失败即返回 `Verify` 错误。
#[cfg(windows)]
pub fn verify_msi(path: &Path) -> Result<VerifyReport> {
    let tables: Vec<String> = msi_api_query(path, "SELECT `Name` FROM `_Tables`")
        .map_err(|error| InstallerError::Verify(format!("open `{}`: {error}", path.display())))?
        .into_iter()
        .filter_map(|mut row| if row.is_empty() { None } else { Some(std::mem::take(&mut row[0])) })
        .collect();
    let mut rows = Vec::new();
    for name in &tables {
        let count = msi_api_query(path, &format!("SELECT * FROM `{name}`"))
            .map_err(|error| InstallerError::Verify(format!("query `{name}`: {error}")))?
            .len();
        rows.push((name.clone(), count));
    }
    Ok(VerifyReport { path: path.to_owned(), tables, rows })
}

/// 非 Windows：如实声明能力缺失。
#[cfg(not(windows))]
pub fn verify_msi(_path: &Path) -> Result<VerifyReport> {
    Err(InstallerError::unsupported("msi-verify"))
}

/// 进程内加载系统 `msi.dll` 执行一条 SQL 查询，返回每行字符串化字段。
/// 只调用 `msi.dll` 导出函数（MSIDBOPEN_READONLY = null），不 spawn 任何 exe。
#[cfg(windows)]
pub(crate) fn msi_api_query(path: &Path, query: &str) -> std::result::Result<Vec<Vec<String>>, String> {
    type MsiHandle = usize;
    type OpenDatabase = unsafe extern "system" fn(*const u16, *const u16, *mut MsiHandle) -> u32;
    type OpenView = unsafe extern "system" fn(MsiHandle, *const u16, *mut MsiHandle) -> u32;
    type ViewExecute = unsafe extern "system" fn(MsiHandle, MsiHandle) -> u32;
    type ViewFetch = unsafe extern "system" fn(MsiHandle, *mut MsiHandle) -> u32;
    type RecordGetString = unsafe extern "system" fn(MsiHandle, u32, *mut u16, *mut u32) -> u32;
    type RecordGetInteger = unsafe extern "system" fn(MsiHandle, u32) -> i32;
    type RecordGetFieldCount = unsafe extern "system" fn(MsiHandle) -> u32;
    type CloseHandle = unsafe extern "system" fn(MsiHandle) -> u32;

    let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(std::iter::once(0)).collect() };
    let ok = 0u32;

    unsafe {
        let library = libloading::Library::new("msi.dll").map_err(|error| error.to_string())?;
        let open: libloading::Symbol<OpenDatabase> = library.get(b"MsiOpenDatabaseW\0").map_err(|e| e.to_string())?;
        let view: libloading::Symbol<OpenView> = library.get(b"MsiDatabaseOpenViewW\0").map_err(|e| e.to_string())?;
        let execute: libloading::Symbol<ViewExecute> = library.get(b"MsiViewExecute\0").map_err(|e| e.to_string())?;
        let fetch: libloading::Symbol<ViewFetch> = library.get(b"MsiViewFetch\0").map_err(|e| e.to_string())?;
        let get_string: libloading::Symbol<RecordGetString> = library.get(b"MsiRecordGetStringW\0").map_err(|e| e.to_string())?;
        let get_integer: libloading::Symbol<RecordGetInteger> = library.get(b"MsiRecordGetInteger\0").map_err(|e| e.to_string())?;
        let field_count: libloading::Symbol<RecordGetFieldCount> = library.get(b"MsiRecordGetFieldCount\0").map_err(|e| e.to_string())?;
        let close: libloading::Symbol<CloseHandle> = library.get(b"MsiCloseHandle\0").map_err(|e| e.to_string())?;

        let mut database: MsiHandle = 0;
        let path_wide = wide(&path.to_string_lossy());
        let open_status = open(path_wide.as_ptr(), std::ptr::null(), &mut database);
        if open_status != ok {
            return Err(format!("MsiOpenDatabaseW status {open_status}"));
        }
        let mut view_handle: MsiHandle = 0;
        let query_wide = wide(query);
        let view_status = view(database, query_wide.as_ptr(), &mut view_handle);
        if view_status != ok {
            close(database);
            return Err(format!("MsiDatabaseOpenViewW status {view_status}"));
        }
        let exec_status = execute(view_handle, 0);
        if exec_status != ok {
            close(view_handle);
            close(database);
            return Err(format!("MsiViewExecute status {exec_status}"));
        }
        let mut rows = Vec::new();
        loop {
            let mut record: MsiHandle = 0;
            let status = fetch(view_handle, &mut record);
            if status != ok {
                break;
            }
            let columns = field_count(record).max(1);
            let mut row = Vec::with_capacity(columns as usize);
            for field in 1..=columns {
                let mut buffer = vec![0u16; 4096];
                let mut length = buffer.len() as u32;
                if get_string(record, field, buffer.as_mut_ptr(), &mut length) == ok {
                    row.push(String::from_utf16_lossy(&buffer[..length as usize]));
                }
                else {
                    row.push(get_integer(record, field).to_string());
                }
            }
            rows.push(row);
            close(record);
        }
        close(view_handle);
        close(database);
        Ok(rows)
    }
}

/// 非 Windows 占位（保持编译面一致）。
#[cfg(not(windows))]
#[allow(dead_code)] // 调用点均为 Windows 门，非 Windows 构建下仅剩占位定义。
pub(crate) fn msi_api_query(_path: &Path, _query: &str) -> std::result::Result<Vec<Vec<String>>, String> {
    Err("msi.dll is Windows-only".into())
}

/// 进程内加载 `ole32.dll` 调 `StgOpenStorageEx` 打开复合文档（不 spawn exe）。
/// 用于区分「容器本身损坏」与「msi.dll 数据库语义失败」。仅诊断用。
#[cfg(windows)]
#[allow(dead_code)] // 诊断用：仅 ignored 的 bisect 测试调用，lib 构建下视为未用。
pub(crate) fn ole32_storage_open(path: &Path) -> std::result::Result<(), u32> {
    type StgOpenStorage =
        unsafe extern "system" fn(*const u16, *mut core::ffi::c_void, u32, *const u16, u32, *mut *mut core::ffi::c_void) -> i32;
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let library = libloading::Library::new("ole32.dll").map_err(|_| 0xFFFFu32)?;
        let open: libloading::Symbol<StgOpenStorage> = library.get(b"StgOpenStorage\0").map_err(|_| 0xFFFFu32)?;
        // STGM_READ=0 | STGM_SHARE_DENY_NONE=0x40。
        let mut storage: *mut core::ffi::c_void = core::ptr::null_mut();
        let hr = open(wide.as_ptr(), core::ptr::null_mut(), 0x40, core::ptr::null(), 0, &mut storage);
        let _ = storage;
        if hr == 0 { Ok(()) } else { Err(hr as u32) }
    }
}

#[cfg(not(windows))]
#[allow(dead_code)] // 调用点均为 Windows 门，非 Windows 构建下仅剩占位定义。
pub(crate) fn ole32_storage_open(_path: &Path) -> std::result::Result<(), u32> {
    Err(0xFFFF)
}
