//! append-only JSONL 落盘、按大小轮转与查询。

use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use thiserror::Error;

use crate::{Entry, Query, sanitize_params};

/// 审计写入或查询的失败。
#[derive(Debug, Error)]
pub enum AuditError {
    /// 文件系统操作失败。
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// 记录序列化失败。
    #[error("{0}")]
    Serialize(#[from] serde_json::Error),
}

/// 审计操作的结果别名。
pub type AuditResult<T> = Result<T, AuditError>;

/// 单个应用的审计记录存储。
///
/// 同一实例内的追加与查询共享一把互斥锁；跨进程追加依赖操作系统 append 语义。
pub struct Store {
    dir: PathBuf,
    stem: String,
    max_bytes: u64,
    lock: Mutex<()>,
}

impl Store {
    /// 以目录、应用名与轮转阈值创建存储。
    ///
    /// `max_bytes` 为 0 时按 1 处理。
    pub fn new(dir: impl Into<PathBuf>, app: &str, max_bytes: u64) -> Self {
        Self { dir: dir.into(), stem: file_stem(app), max_bytes: max_bytes.max(1), lock: Mutex::new(()) }
    }

    /// 当前（未轮转）审计文件路径。
    pub fn path(&self) -> PathBuf {
        self.dir.join(format!("{}.jsonl", self.stem))
    }

    /// 追加一条记录；参数在写入前再次脱敏。
    pub fn append(&self, entry: &Entry) -> AuditResult<()> {
        let mut record = entry.clone();
        record.params = sanitize_params(record.params);
        record.targets.summary = crate::sanitize::sanitize_text(&record.targets.summary);
        let line = serde_json::to_string(&record)?;
        self.append_line(&line)
    }

    /// 追加一行已序列化的记录（必须为单行 JSON）。
    pub fn append_line(&self, line: &str) -> AuditResult<()> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        fs::create_dir_all(&self.dir)?;
        let path = self.path();
        self.rotate_if_needed(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut buffer = Vec::with_capacity(line.len() + 1);
        buffer.extend_from_slice(line.as_bytes());
        buffer.push(b'\n');
        file.write_all(&buffer)?;
        file.flush()?;
        Ok(())
    }

    /// 按条件读取记录，返回最新优先；轮转文件一并读取，损坏行跳过。
    pub fn query(&self, filter: &Query) -> AuditResult<Vec<Entry>> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut records = Vec::new();
        for (_, path) in self.files()? {
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            for line in text.lines() {
                let Ok(entry) = serde_json::from_str::<Entry>(line.trim())
                else {
                    continue;
                };
                if filter.matches(&entry) {
                    records.push(entry);
                }
            }
        }
        records.sort_by_key(|entry| entry.timestamp_ms);
        if let Some(limit) = filter.limit
            && records.len() > limit
        {
            records.drain(0..records.len() - limit);
        }
        records.reverse();
        Ok(records)
    }

    fn rotate_if_needed(&self, current: &Path) -> AuditResult<()> {
        let size = match fs::metadata(current) {
            Ok(meta) => meta.len(),
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        if size < self.max_bytes {
            return Ok(());
        }
        let sequence = self.next_sequence()?;
        let target = self.dir.join(format!("{}.{}.jsonl", self.stem, sequence));
        fs::rename(current, target)?;
        Ok(())
    }

    fn next_sequence(&self) -> AuditResult<u64> {
        let mut max = 0u64;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str()
            else {
                continue;
            };
            if let Some(sequence) = rotated_sequence(name, &self.stem) {
                max = max.max(sequence);
            }
        }
        Ok(max + 1)
    }

    /// 列出全部审计文件，轮转文件按序号升序，当前文件排最后。
    fn files(&self) -> AuditResult<Vec<(Option<u64>, PathBuf)>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            if !entry.path().is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str()
            else {
                continue;
            };
            let sequence = if name == format!("{}.jsonl", self.stem) {
                None
            }
            else {
                match rotated_sequence(name, &self.stem) {
                    Some(sequence) => Some(sequence),
                    None => continue,
                }
            };
            files.push((sequence, entry.path()));
        }
        files.sort_by_key(|(sequence, _)| match sequence {
            Some(sequence) => (*sequence, 0u8),
            None => (u64::MAX, 1u8),
        });
        Ok(files)
    }
}

fn rotated_sequence(name: &str, stem: &str) -> Option<u64> {
    name.strip_prefix(stem)?.strip_prefix('.')?.strip_suffix(".jsonl")?.parse().ok()
}

fn file_stem(app: &str) -> String {
    let cleaned: String = app.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        return "app".into();
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stem_keeps_only_safe_characters() {
        assert_eq!(file_stem("open-cleaner"), "open-cleaner");
        assert_eq!(file_stem("a/b:c"), "a_b_c");
        assert_eq!(file_stem(".."), "app");
    }

    #[test]
    fn rotated_sequence_recognizes_rotated_files() {
        assert_eq!(rotated_sequence("demo.3.jsonl", "demo"), Some(3));
        assert_eq!(rotated_sequence("demo.jsonl", "demo"), None);
        assert_eq!(rotated_sequence("demo.x.jsonl", "demo"), None);
        assert_eq!(rotated_sequence("other.3.jsonl", "demo"), None);
    }
}
