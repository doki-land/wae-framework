//! MSI 逻辑表模型 → CFB 流字节编码。
//!
//! 编码遵循 [MSI] §3.8：`_Tables`/`_Columns` 元数据流、ANSI 字符串池
//! （块 = `u32 blockSize` 含自身 + `u32 codepage` + NUL 结尾串）、行 =
//! `u16 rowSize`（含自身）+ `u16 fieldCount` + `u16 × 字段宽` + 字段数据，
//! 文本字段为 12 字节串引用（stream/block/index），整数为 `u32 LE`，
//! NULL 字段宽 `0xFFFF`，流尾 `u16 0` 终止行。
//!
//! 列的 `Type` 值（含宽度与可空/主键/本地化标志）由 schema spike 从真实
//! MSI 经 `MsiOpenDatabase` SQL 视图钉死，禁止凭记忆改动。

use std::collections::HashMap;

use crate::error::{InstallerError, Result};

/// 单元格：文本（进字符串池）、整数、或 NULL。
#[derive(Debug, Clone)]
pub(crate) enum Cell {
    Text(String),
    Int(u32),
    Null,
}

/// 一列的定义：名字与 Type 值。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Column {
    /// 列名。
    pub name: &'static str,
    /// [MSI] `Type` 列值（低位宽度 + 高位标志）。
    pub kind: u32,
}

/// 一张逻辑表。
#[derive(Debug)]
pub(crate) struct Table {
    /// 表名（同时是 CFB 流名）。
    pub name: &'static str,
    /// 列定义。
    pub columns: &'static [Column],
    /// 行数据。
    pub rows: Vec<Vec<Cell>>,
}

/// 字符串池：块列表，每块是 NUL 结尾 ANSI 串。
#[derive(Debug)]
pub(crate) struct StringPool {
    /// 已收录串 → (block, index)。
    seen: HashMap<String, (u32, u32)>,
    /// 块：每块的字符串表。
    blocks: Vec<Vec<String>>,
    /// 当前块内字符串表。
    current: Vec<String>,
    /// 当前块累计字节（含 8 字节块头）。
    current_bytes: usize,
}

impl Default for StringPool {
    fn default() -> Self {
        Self { seen: HashMap::new(), blocks: Vec::new(), current: Vec::new(), current_bytes: 8 }
    }
}

/// 单块上限：64 KB（含头）。
const BLOCK_LIMIT: usize = 64 * 1024;
/// 代码页 1252（ANSI Latin-1）。
const CODEPAGE: u32 = 1252;

impl StringPool {
    /// 收录文本，返回引用坐标。
    pub(crate) fn intern(&mut self, text: &str) -> (u32, u32) {
        if let Some(pos) = self.seen.get(text) {
            return *pos;
        }
        // 新串放不下就封块。
        if self.current_bytes + text.len() + 1 > BLOCK_LIMIT {
            self.flush();
        }
        let block = self.blocks.len() as u32;
        let index = self.current.len() as u32;
        self.current.push(text.to_owned());
        self.current_bytes += text.len() + 1;
        self.seen.insert(text.to_owned(), (block, index));
        (block, index)
    }

    fn flush(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let block = std::mem::take(&mut self.current);
        self.current_bytes = 8;
        self.blocks.push(block);
    }

    /// 产出 `_StringPool` 与 `_StringData` 字节。
    pub(crate) fn finish(mut self) -> (Vec<u8>, Vec<u8>) {
        self.flush();
        let mut pool = Vec::new();
        let mut data = Vec::new();
        for block in &self.blocks {
            data.extend_from_slice(&(pool.len() as u32).to_le_bytes());
            let body: usize = block.iter().map(|text| text.len() + 1).sum();
            let size = (8 + body) as u32;
            pool.extend_from_slice(&size.to_le_bytes());
            pool.extend_from_slice(&CODEPAGE.to_le_bytes());
            for text in block {
                pool.extend_from_slice(text.as_bytes());
                pool.push(0);
            }
        }
        (pool, data)
    }
}

/// 编码一张表的行流字节。
pub(crate) fn encode_rows(table: &Table, pool: &mut StringPool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for row in &table.rows {
        if row.len() != table.columns.len() {
            return Err(InstallerError::Layout(format!("table `{}` row arity {}", table.name, row.len())));
        }
        let mut widths = Vec::with_capacity(row.len());
        let mut body = Vec::new();
        for cell in row {
            match cell {
                Cell::Null => widths.push(0xFFFFu16),
                Cell::Int(value) => {
                    widths.push(4);
                    body.extend_from_slice(&value.to_le_bytes());
                }
                Cell::Text(text) => {
                    let (block, index) = pool.intern(text);
                    widths.push(12);
                    body.extend_from_slice(&0u32.to_le_bytes()); // stream
                    body.extend_from_slice(&block.to_le_bytes());
                    body.extend_from_slice(&index.to_le_bytes());
                }
            }
        }
        let row_size = 2u16 + 2 + 2 * table.columns.len() as u16 + body.len() as u16;
        out.extend_from_slice(&row_size.to_le_bytes());
        out.extend_from_slice(&(table.columns.len() as u16).to_le_bytes());
        for width in widths {
            out.extend_from_slice(&width.to_le_bytes());
        }
        out.extend_from_slice(&body);
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

/// 编码 `_Tables` 流（单列 Name）。
pub(crate) fn encode_tables(names: &[&'static str], pool: &mut StringPool) -> Result<Vec<u8>> {
    let table = Table {
        name: "_Tables",
        columns: &[Column { name: "Name", kind: KIND_S72_PK }],
        rows: names.iter().map(|name| vec![Cell::Text((*name).to_owned())]).collect(),
    };
    encode_rows(&table, pool)
}

/// 编码 `_Columns` 流。
pub(crate) fn encode_columns(tables: &[&Table], pool: &mut StringPool) -> Result<Vec<u8>> {
    let mut rows = Vec::new();
    for table in tables {
        for (index, column) in table.columns.iter().enumerate() {
            rows.push(vec![
                Cell::Text(table.name.to_owned()),
                Cell::Int(index as u32 + 1),
                Cell::Text(column.name.to_owned()),
                Cell::Int(column.kind),
            ]);
        }
    }
    let meta = Table {
        name: "_Columns",
        columns: &[
            Column { name: "Table", kind: KIND_S72_PK },
            Column { name: "Number", kind: KIND_I2_PK },
            Column { name: "Name", kind: KIND_S72_PK },
            Column { name: "Type", kind: KIND_I4 },
        ],
        rows,
    };
    encode_rows(&meta, pool)
}

/// Type 值常量（spike 自真实 MSI 的 `_Columns`，禁止改动）。
pub(crate) const KIND_S72_PK: u32 = 11592;
pub(crate) const KIND_S72: u32 = 3400;
pub(crate) const KIND_S72_NULL: u32 = 7496;
pub(crate) const KIND_S38_NULL: u32 = 7462;
pub(crate) const KIND_S64_PK: u32 = 11558;
pub(crate) const KIND_S255_NULL: u32 = 7679;
pub(crate) const KIND_S255_LOC: u32 = 4095;
pub(crate) const KIND_S64_LOC_NULL: u32 = 8000;
pub(crate) const KIND_S255_LOC_NULL: u32 = 8191;
pub(crate) const KIND_S20_NULL: u32 = 7444;
pub(crate) const KIND_S64_NULL: u32 = 7456;
pub(crate) const KIND_I2: u32 = 1282;
pub(crate) const KIND_I2_NULL: u32 = 5378;
pub(crate) const KIND_I2_PK: u32 = 9474;
pub(crate) const KIND_I4: u32 = 260;

/// Property 表列定义。
pub(crate) const PROPERTY_COLUMNS: &[Column] =
    &[Column { name: "Property", kind: KIND_S72_PK }, Column { name: "Value", kind: KIND_S255_NULL }];
/// Directory 表列定义。
pub(crate) const DIRECTORY_COLUMNS: &[Column] = &[
    Column { name: "Directory", kind: KIND_S72_PK },
    Column { name: "Directory_Parent", kind: KIND_S72_NULL },
    Column { name: "DefaultDir", kind: KIND_S255_LOC },
];
/// Component 表列定义。
pub(crate) const COMPONENT_COLUMNS: &[Column] = &[
    Column { name: "Component", kind: KIND_S72_PK },
    Column { name: "ComponentId", kind: KIND_S38_NULL },
    Column { name: "Directory_", kind: KIND_S72 },
    Column { name: "Attributes", kind: KIND_I2 },
    Column { name: "Condition", kind: KIND_S255_NULL },
    Column { name: "KeyPath", kind: KIND_S72_NULL },
];
/// Feature 表列定义。
pub(crate) const FEATURE_COLUMNS: &[Column] = &[
    Column { name: "Feature", kind: KIND_S64_PK },
    Column { name: "Feature_Parent", kind: KIND_S38_NULL },
    Column { name: "Title", kind: KIND_S64_LOC_NULL },
    Column { name: "Description", kind: KIND_S255_LOC_NULL },
    Column { name: "Display", kind: KIND_I2_NULL },
    Column { name: "Level", kind: KIND_I2 },
    Column { name: "Directory_", kind: KIND_S72_NULL },
    Column { name: "Attributes", kind: KIND_I2 },
];
/// FeatureComponents 表列定义。
pub(crate) const FEATURE_COMPONENTS_COLUMNS: &[Column] =
    &[Column { name: "Feature_", kind: KIND_S64_PK }, Column { name: "Component_", kind: KIND_S72_PK }];
/// File 表列定义。
pub(crate) const FILE_COLUMNS: &[Column] = &[
    Column { name: "File", kind: KIND_S72_PK },
    Column { name: "Component_", kind: KIND_S72 },
    Column { name: "FileName", kind: KIND_S255_LOC },
    Column { name: "FileSize", kind: KIND_I4 },
    Column { name: "Version", kind: KIND_S72_NULL },
    Column { name: "Language", kind: KIND_S20_NULL },
    Column { name: "Attributes", kind: KIND_I2_NULL },
    Column { name: "Sequence", kind: KIND_I4 },
];
/// Media 表列定义。
pub(crate) const MEDIA_COLUMNS: &[Column] = &[
    Column { name: "DiskId", kind: KIND_I2_PK },
    Column { name: "LastSequence", kind: KIND_I4 },
    Column { name: "DiskPrompt", kind: KIND_S64_LOC_NULL },
    Column { name: "Cabinet", kind: KIND_S255_NULL },
    Column { name: "VolumeLabel", kind: KIND_S64_NULL },
    Column { name: "Source", kind: KIND_S72_NULL },
];
/// InstallExecuteSequence 表列定义。
pub(crate) const SEQUENCE_COLUMNS: &[Column] = &[
    Column { name: "Action", kind: KIND_S72_PK },
    Column { name: "Condition", kind: KIND_S255_NULL },
    Column { name: "Sequence", kind: KIND_I2_NULL },
];
/// Upgrade 表列定义。
pub(crate) const UPGRADE_COLUMNS: &[Column] = &[
    Column { name: "UpgradeCode", kind: KIND_S38_NULL },
    Column { name: "Ref", kind: KIND_S72_PK },
    Column { name: "VersionMin", kind: KIND_S20_NULL },
    Column { name: "VersionMax", kind: KIND_S20_NULL },
    Column { name: "Language", kind: KIND_S20_NULL },
    Column { name: "Attributes", kind: KIND_I2_NULL },
    Column { name: "RemoveAttributes", kind: KIND_I2_NULL },
];
/// Icon 表列定义。
pub(crate) const ICON_COLUMNS: &[Column] = &[Column { name: "Name", kind: KIND_S72_PK }, Column { name: "Data", kind: KIND_I4 }];
/// Shortcut 表列定义。
pub(crate) const SHORTCUT_COLUMNS: &[Column] = &[
    Column { name: "Shortcut", kind: KIND_S72_PK },
    Column { name: "Component_", kind: KIND_S72 },
    Column { name: "Name", kind: KIND_S255_LOC },
    Column { name: "Target", kind: KIND_S255_LOC },
    Column { name: "Arguments", kind: KIND_S255_LOC_NULL },
    Column { name: "Description", kind: KIND_S255_LOC_NULL },
    Column { name: "DirProperty", kind: KIND_S72 },
    Column { name: "WkDir", kind: KIND_S72_NULL },
    Column { name: "ShowCmd", kind: KIND_I2 },
    Column { name: "Icon", kind: KIND_S72_NULL },
    Column { name: "IconIndex", kind: KIND_I2_NULL },
    Column { name: "Hotkey", kind: KIND_I2_NULL },
    Column { name: "Name_Localizable", kind: KIND_S255_LOC },
    Column { name: "Description_Localizable", kind: KIND_S255_LOC_NULL },
];

#[cfg(test)]
pub(crate) fn pool_strings(pool: &[u8]) -> Vec<Vec<String>> {
    let mut blocks = Vec::new();
    let mut at = 0usize;
    while at + 8 <= pool.len() {
        let size = u32::from_le_bytes(pool[at..at + 4].try_into().unwrap()) as usize;
        if size < 8 || at + size > pool.len() {
            break;
        }
        let mut strings = Vec::new();
        let mut cursor = at + 8;
        while cursor < at + size {
            let end = pool[cursor..at + size].iter().position(|byte| *byte == 0).map(|p| cursor + p).unwrap_or(at + size);
            strings.push(String::from_utf8_lossy(&pool[cursor..end]).into_owned());
            cursor = end + 1;
        }
        blocks.push(strings);
        at += size;
    }
    blocks
}

#[cfg(test)]
pub(crate) fn decode_rows(data: &[u8], pool: &[u8]) -> Vec<Vec<Cell>> {
    let blocks = pool_strings(pool);
    let mut rows = Vec::new();
    let mut at = 0usize;
    while at + 4 <= data.len() {
        let row_size = u16::from_le_bytes(data[at..at + 2].try_into().unwrap()) as usize;
        if row_size == 0 {
            break;
        }
        let fields = u16::from_le_bytes(data[at + 2..at + 4].try_into().unwrap()) as usize;
        let mut widths = Vec::with_capacity(fields);
        for index in 0..fields {
            widths.push(u16::from_le_bytes(data[at + 4 + index * 2..at + 6 + index * 2].try_into().unwrap()));
        }
        let mut cursor = at + 4 + fields * 2;
        let mut row = Vec::with_capacity(fields);
        for width in widths {
            match width {
                0xFFFF => row.push(Cell::Null),
                4 => {
                    row.push(Cell::Int(u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap())));
                    cursor += 4;
                }
                12 => {
                    let block = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
                    let index = u32::from_le_bytes(data[cursor + 8..cursor + 12].try_into().unwrap()) as usize;
                    row.push(Cell::Text(blocks[block][index].clone()));
                    cursor += 12;
                }
                other => {
                    cursor += other as usize;
                    row.push(Cell::Null);
                }
            }
        }
        rows.push(row);
        at += row_size;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_blocks_and_refs_round_trip() {
        let mut pool = StringPool::default();
        let a = pool.intern("Alpha");
        let b = pool.intern("Bravo");
        let again = pool.intern("Alpha");
        assert_eq!(a, again);
        assert_eq!(a, (0, 0));
        assert_eq!(b, (0, 1));
        let (text, data) = pool.finish();
        // 块头 = 8 + 6 + 6 = 20。
        assert_eq!(&text[0..4], &20u32.to_le_bytes());
        assert_eq!(&text[4..8], &1252u32.to_le_bytes());
        assert_eq!(&text[8..14], b"Alpha\0");
        assert_eq!(&text[14..20], b"Bravo\0");
        assert_eq!(data, 0u32.to_le_bytes());
    }

    #[test]
    fn pool_splits_at_64k() {
        let mut pool = StringPool::default();
        let big = "x".repeat(60 * 1024);
        let first = pool.intern(&big);
        let second = pool.intern("tail");
        assert_eq!(first, (0, 0));
        assert_eq!(second, (0, 1));
        let big2 = "y".repeat(60 * 1024);
        let third = pool.intern(&big2);
        assert_eq!(third, (1, 0));
        let (text, data) = pool.finish();
        assert_eq!(u32::from_le_bytes(data[0..4].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), (8 + big.len() + 1 + 5) as u32);
        assert_eq!(text.len(), 8 + big.len() + 1 + 5 + 8 + big2.len() + 1);
    }

    #[test]
    fn rows_encode_with_terminator() {
        let mut pool = StringPool::default();
        let table = Table {
            name: "Property",
            columns: PROPERTY_COLUMNS,
            rows: vec![
                vec![Cell::Text("ProductName".into()), Cell::Text("MyJarvis".into())],
                vec![Cell::Text("ALLUSERS".into()), Cell::Int(1)],
            ],
        };
        let bytes = encode_rows(&table, &mut pool).unwrap();
        // 行1：size = 2+2+4 + 12+12 = 32。
        assert_eq!(u16::from_le_bytes(bytes[0..2].try_into().unwrap()), 32);
        assert_eq!(u16::from_le_bytes(bytes[2..4].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 12);
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 12);
        // 行2：size = 2+2+4 + 12+4 = 24。
        let row2 = &bytes[32..];
        assert_eq!(u16::from_le_bytes(row2[0..2].try_into().unwrap()), 24);
        assert_eq!(u16::from_le_bytes(row2[4..6].try_into().unwrap()), 12);
        assert_eq!(u16::from_le_bytes(row2[6..8].try_into().unwrap()), 4);
        // 流尾终止行。
        let end = &bytes[32 + 24..];
        assert_eq!(end, &[0, 0]);
    }

    #[test]
    fn null_cell_uses_wide_marker() {
        let mut pool = StringPool::default();
        let table = Table { name: "T", columns: SEQUENCE_COLUMNS, rows: vec![vec![Cell::Text("Foo".into()), Cell::Null, Cell::Int(100)]] };
        let bytes = encode_rows(&table, &mut pool).unwrap();
        // 宽度数组：`[4..6]`=Text 12，`[6..8]`=Null 0xFFFF，`[8..10]`=Int 4。
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 0xFFFF);
    }
}
