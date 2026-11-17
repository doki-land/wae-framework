//! CFB v4（MS-CFB / OLE 复合文档）writer：4096 字节扇区、64 字节 mini 扇区、
//! miniFAT 与内联 DIFAT（≤109 个 FAT 扇区，产物上限约 448 MB）。
//!
//! MSI 数据库就是这个容器：`_Tables`、`_StringPool` 等小流（<4096 字节）
//! 进 mini 扇区，内嵌 cabinet 大流走常规扇区链。扇区大小必须是 4096：
//! 本机 `C:\Windows\Installer\*.msi` 实测 `sectorShift=12`，512 字节的 v3
//! 容器会被 `msi.dll` 以 `ERROR_OPEN_FAILED`（110）拒收。目录树按 MS-CFB
//! §2.6.4.1 排序（名称长度升序，再大小写不敏感）插入为真红黑树——ole32
//! 按名查找走树下降，退化的兄弟链会让查找落空。
//!
//! v0 在内存中组装完整文件字节（构建期工具可接受）。`add_stream` 的
//! `path` 用 `/` 分隔存储层级，缺失的中间段自动成为存储。

use std::io::Write;

use crate::error::{InstallerError, Result};

/// 常规扇区大小（字节）。MSI 数据库固定 4096（CFB v4）。
const SECTOR: usize = 4096;
/// mini 扇区大小（字节）。
const MINI: usize = 64;
/// 小于该值的流进 mini 扇区（`maxMiniStreamSize`）。
const MAX_MINI: usize = 4096;
/// 目录项大小（字节）。
const ENTRY: usize = 128;
/// 头部内联 DIFAT 项数。
const DIFAT_INLINE: usize = 109;

const NOSTREAM: u32 = 0xFFFF_FFFF;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;
const FATSECT: u32 = 0xFFFF_FFFD;

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// 目录项类型。
const TYPE_UNKNOWN: u8 = 0;
const TYPE_STORAGE: u8 = 1;
const TYPE_STREAM: u8 = 2;
const TYPE_ROOT: u8 = 5;

/// 流在容器中的位置计划。
#[derive(Debug, Clone, Copy)]
enum Placement {
    /// 空流或存储：无数据扇区。
    None,
    /// mini 流：起始 mini 扇区号 + mini 扇区数。
    Mini { start: u32, sectors: u32 },
    /// 常规流：起始扇区号 + 扇区数。
    Regular { start: u32, sectors: u32 },
}

#[derive(Debug)]
enum Kind {
    Root,
    Storage,
    Stream(Vec<u8>),
}

#[derive(Debug)]
struct Node {
    name: String,
    kind: Kind,
    children: Vec<usize>,
}

/// CFB 文件构造器；下标即目录项 id（0 恒为 `Root Entry`）。
#[derive(Debug, Default)]
pub struct Cfb {
    nodes: Vec<Node>,
}

impl Cfb {
    /// 新建只含 `Root Entry` 的容器。
    pub fn new() -> Self {
        Self { nodes: vec![Node { name: "Root Entry".into(), kind: Kind::Root, children: Vec::new() }] }
    }

    /// 追加一个流；`path` 以 `/` 分隔。
    pub fn add_stream(&mut self, path: &str, data: Vec<u8>) -> Result<()> {
        let segments: Vec<&str> = path.split('/').collect();
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err(InstallerError::Layout(format!("invalid CFB path `{path}`")));
        }
        let mut parent = 0usize;
        let (leaf, intermediates) = segments.split_last().expect("non-empty segments");
        for segment in intermediates {
            parent = self.child_storage(parent, segment);
        }
        if self.nodes[parent].children.iter().any(|id| self.nodes[*id].name == *leaf) {
            return Err(InstallerError::Layout(format!("duplicate CFB stream `{path}`")));
        }
        self.nodes.push(Node { name: (*leaf).to_owned(), kind: Kind::Stream(data), children: Vec::new() });
        let new_id = self.nodes.len() - 1;
        self.nodes[parent].children.push(new_id);
        Ok(())
    }

    fn child_storage(&mut self, parent: usize, name: &str) -> usize {
        if let Some(id) =
            self.nodes[parent].children.iter().copied().find(|id| matches!(self.nodes[*id].kind, Kind::Storage) && self.nodes[*id].name == name)
        {
            return id;
        }
        self.nodes.push(Node { name: name.to_owned(), kind: Kind::Storage, children: Vec::new() });
        let id = self.nodes.len() - 1;
        self.nodes[parent].children.push(id);
        id
    }

    /// 组装完整文件字节。
    pub fn finish(&self) -> Result<Vec<u8>> {
        // 1. 分类：mini（<4096 且非空）与常规流。
        let mut placements: Vec<Placement> = vec![Placement::None; self.nodes.len()];
        let mut mini_sector_cursor = 0u32;
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            if let Kind::Stream(data) = &node.kind {
                if data.is_empty() {
                    placements[id] = Placement::None;
                }
                else if data.len() < MAX_MINI {
                    let sectors = data.len().div_ceil(MINI) as u32;
                    placements[id] = Placement::Mini { start: mini_sector_cursor, sectors };
                    mini_sector_cursor += sectors;
                }
            }
        }
        let mini_sector_total = mini_sector_cursor as usize;
        let mini_stream_bytes = mini_sector_total * MINI;
        let mini_stream_sectors = mini_stream_bytes.div_ceil(SECTOR);
        let regular_sectors_total: usize = self
            .nodes
            .iter()
            .skip(1)
            .filter_map(|node| match &node.kind {
                Kind::Stream(data) if data.len() >= MAX_MINI => Some(data.len().div_ceil(SECTOR)),
                _ => None,
            })
            .sum();
        let dir_sectors = (self.nodes.len() * ENTRY).div_ceil(SECTOR);
        let minifat_sectors = if mini_sector_total > 0 { (mini_sector_total * 4).div_ceil(SECTOR) } else { 0 };

        // 2. FAT 数量定点迭代（FAT 自身也要被覆盖）。
        let payload_sectors = regular_sectors_total + mini_stream_sectors + dir_sectors + minifat_sectors;
        let mut fat_sectors = 0usize;
        loop {
            let next = (payload_sectors + fat_sectors + 1).div_ceil(SECTOR / 4);
            if next == fat_sectors {
                break;
            }
            fat_sectors = next;
        }
        if fat_sectors > DIFAT_INLINE {
            return Err(InstallerError::Layout(format!("CFB needs {fat_sectors} FAT sectors; inline DIFAT caps at {DIFAT_INLINE} (~448 MB)")));
        }

        // 3. 扇区号布局：[FAT][目录][常规流...][mini 流][miniFAT]。
        let fat_start = 0u32;
        let dir_start = fat_sectors as u32;
        let mut cursor = dir_start + dir_sectors as u32;
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            if let Kind::Stream(data) = &node.kind
                && data.len() >= MAX_MINI
            {
                let sectors = data.len().div_ceil(SECTOR) as u32;
                placements[id] = Placement::Regular { start: cursor, sectors };
                cursor += sectors;
            }
        }
        let mini_stream_start = cursor;
        cursor += mini_stream_sectors as u32;
        let minifat_start = cursor;
        cursor += minifat_sectors as u32;

        // 4. FAT 数组。
        let total_sectors = cursor as usize;
        let mut fat = vec![FREESECT; fat_sectors * (SECTOR / 4)];
        for id in fat_start..fat_start + fat_sectors as u32 {
            fat[id as usize] = FATSECT;
        }
        link(&mut fat, dir_start, dir_sectors as u32);
        if mini_stream_sectors > 0 {
            link(&mut fat, mini_stream_start, mini_stream_sectors as u32);
        }
        if minifat_sectors > 0 {
            link(&mut fat, minifat_start, minifat_sectors as u32);
        }
        for placement in placements.iter().skip(1) {
            if let Placement::Regular { start, sectors } = placement {
                link(&mut fat, *start, *sectors);
            }
        }

        // 5. miniFAT 数组与 mini 数据。
        let mut minifat = vec![FREESECT; minifat_sectors * (SECTOR / 4)];
        let mut mini_bytes = vec![0u8; mini_stream_bytes];
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            if let Kind::Stream(data) = &node.kind
                && let Placement::Mini { start, sectors } = placements[id]
            {
                link(&mut minifat, start, sectors);
                let at = start as usize * MINI;
                mini_bytes[at..at + data.len()].copy_from_slice(data);
            }
        }

        // 6. 目录项：每个存储的子项按 MS-CFB 排序插入为红黑树。
        let trees = build_dir_trees(&self.nodes);
        let mut dir = vec![0u8; dir_sectors * SECTOR];
        // Root：start = mini 流链首，size = mini 流总字节。
        let root_start = if mini_stream_sectors > 0 { mini_stream_start } else { ENDOFCHAIN };
        write_entry(&mut dir, 0, "Root Entry", TYPE_ROOT, trees.roots[0], root_start, mini_stream_bytes as u64, NOSTREAM, NOSTREAM, BLACK);
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            let (kind, start, size) = match &node.kind {
                Kind::Storage => (TYPE_STORAGE, ENDOFCHAIN, 0u64),
                Kind::Stream(data) => match placements[id] {
                    Placement::None => (TYPE_STREAM, ENDOFCHAIN, 0),
                    Placement::Mini { start, .. } => (TYPE_STREAM, start, data.len() as u64),
                    Placement::Regular { start, .. } => (TYPE_STREAM, start, data.len() as u64),
                },
                Kind::Root => unreachable!("root handled above"),
            };
            write_entry(&mut dir, id, &node.name, kind, trees.roots[id], start, size, trees.left[id], trees.right[id], trees.color[id]);
        }

        // 7. 组装文件。
        let mut file = vec![0u8; SECTOR * (1 + total_sectors)];
        let minifat_header = if minifat_sectors > 0 { minifat_start } else { ENDOFCHAIN };
        write_header(&mut file, fat_sectors as u32, dir_start, minifat_header, minifat_sectors as u32, fat_start, dir_sectors as u32);
        write_u32_table(&mut file, fat_start as usize, &fat);
        copy_sectors(&mut file, dir_start as usize, &dir);
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            if let Kind::Stream(data) = &node.kind
                && let Placement::Regular { start, .. } = placements[id]
            {
                copy_sectors(&mut file, start as usize, data);
            }
        }
        if mini_stream_sectors > 0 {
            copy_sectors(&mut file, mini_stream_start as usize, &mini_bytes);
        }
        if minifat_sectors > 0 {
            write_u32_table(&mut file, minifat_start as usize, &minifat);
        }
        Ok(file)
    }

    /// 组装并写入 sink，返回字节数。
    pub fn write_to<W: Write>(&self, mut sink: W) -> Result<u64> {
        let bytes = self.finish()?;
        sink.write_all(&bytes).map_err(InstallerError::Io)?;
        Ok(bytes.len() as u64)
    }
}

/// 在 `table` 中把 `start..start+count` 写成连续链，尾项 ENDOFCHAIN。
fn link(table: &mut [u32], start: u32, count: u32) {
    for index in 0..count {
        let at = (start + index) as usize;
        table[at] = if index + 1 == count { ENDOFCHAIN } else { start + index + 1 };
    }
}

fn copy_sectors(file: &mut [u8], start: usize, data: &[u8]) {
    let at = SECTOR * (1 + start);
    file[at..at + data.len()].copy_from_slice(data);
}

/// 把 u32 表按小端写入从扇区 `start` 开始的区域。
fn write_u32_table(file: &mut [u8], start: usize, table: &[u32]) {
    let at = SECTOR * (1 + start);
    for (index, value) in table.iter().enumerate() {
        file[at + index * 4..at + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// 目录颜色（MS-CFB：0=RED，1=BLACK）。
const RED: u8 = 0;
const BLACK: u8 = 1;

/// MS-CFB §2.6.4.1 目录排序：先按名称长度（UTF-16 单元数）升序，
/// 再按大小写不敏感逐单元比较，最后按原始单元序。
fn name_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let ua: Vec<u16> = a.encode_utf16().collect();
    let ub: Vec<u16> = b.encode_utf16().collect();
    ua.len().cmp(&ub.len()).then_with(|| {
        for (x, y) in ua.iter().zip(&ub) {
            let (xf, yf) = (fold_unit(*x), fold_unit(*y));
            if xf != yf {
                return xf.cmp(&yf);
            }
        }
        ua.cmp(&ub)
    })
}

fn fold_unit(unit: u16) -> u16 {
    if (0x41..=0x5A).contains(&unit) { unit + 0x20 } else { unit }
}

/// 一个存储下所有子项的红黑树链接（下标 = 全局目录项 id）。
struct DirTrees {
    left: Vec<u32>,
    right: Vec<u32>,
    color: Vec<u8>,
    /// `roots[p]` = 存储 `p` 子项树的树根 id（无子项则 `NOSTREAM`）。
    roots: Vec<u32>,
}

/// 按插入序把每个存储的子项插入红黑树（CLRS INSERT-FIXUP，NOSTREAM 作哨兵）。
fn build_dir_trees(nodes: &[Node]) -> DirTrees {
    use std::cmp::Ordering;
    let n = nodes.len();
    let mut left = vec![NOSTREAM; n];
    let mut right = vec![NOSTREAM; n];
    let mut color = vec![BLACK; n];
    let mut parent = vec![NOSTREAM; n];
    let mut roots = vec![NOSTREAM; n];
    for p in 0..n {
        let mut root = NOSTREAM;
        for &child in &nodes[p].children {
            let mut z = child as u32;
            let mut y = NOSTREAM;
            let mut x = root;
            while x != NOSTREAM {
                y = x;
                x = if name_cmp(&nodes[z as usize].name, &nodes[x as usize].name) == Ordering::Less {
                    left[x as usize]
                }
                else {
                    right[x as usize]
                };
            }
            parent[z as usize] = y;
            if y == NOSTREAM {
                root = z;
            }
            else if name_cmp(&nodes[z as usize].name, &nodes[y as usize].name) == Ordering::Less {
                left[y as usize] = z;
            }
            else {
                right[y as usize] = z;
            }
            color[z as usize] = RED;
            while parent[z as usize] != NOSTREAM && color[parent[z as usize] as usize] == RED {
                let gp = parent[parent[z as usize] as usize];
                if parent[z as usize] == left[gp as usize] {
                    let uncle = right[gp as usize];
                    if uncle != NOSTREAM && color[uncle as usize] == RED {
                        color[parent[z as usize] as usize] = BLACK;
                        color[uncle as usize] = BLACK;
                        color[gp as usize] = RED;
                        z = gp;
                    }
                    else {
                        if z == right[parent[z as usize] as usize] {
                            z = parent[z as usize];
                            rotate_left(&mut root, z, &mut left, &mut right, &mut parent);
                        }
                        color[parent[z as usize] as usize] = BLACK;
                        color[gp as usize] = RED;
                        rotate_right(&mut root, gp, &mut left, &mut right, &mut parent);
                    }
                }
                else {
                    let uncle = left[gp as usize];
                    if uncle != NOSTREAM && color[uncle as usize] == RED {
                        color[parent[z as usize] as usize] = BLACK;
                        color[uncle as usize] = BLACK;
                        color[gp as usize] = RED;
                        z = gp;
                    }
                    else {
                        if z == left[parent[z as usize] as usize] {
                            z = parent[z as usize];
                            rotate_right(&mut root, z, &mut left, &mut right, &mut parent);
                        }
                        color[parent[z as usize] as usize] = BLACK;
                        color[gp as usize] = RED;
                        rotate_left(&mut root, gp, &mut left, &mut right, &mut parent);
                    }
                }
            }
            color[root as usize] = BLACK;
        }
        roots[p] = root;
    }
    DirTrees { left, right, color, roots }
}

fn rotate_left(root: &mut u32, x: u32, left: &mut [u32], right: &mut [u32], parent: &mut [u32]) {
    let y = right[x as usize];
    right[x as usize] = left[y as usize];
    if left[y as usize] != NOSTREAM {
        parent[left[y as usize] as usize] = x;
    }
    parent[y as usize] = parent[x as usize];
    if parent[x as usize] == NOSTREAM {
        *root = y;
    }
    else if x == left[parent[x as usize] as usize] {
        left[parent[x as usize] as usize] = y;
    }
    else {
        right[parent[x as usize] as usize] = y;
    }
    left[y as usize] = x;
    parent[x as usize] = y;
}

fn rotate_right(root: &mut u32, y: u32, left: &mut [u32], right: &mut [u32], parent: &mut [u32]) {
    let x = left[y as usize];
    left[y as usize] = right[x as usize];
    if right[x as usize] != NOSTREAM {
        parent[right[x as usize] as usize] = y;
    }
    parent[x as usize] = parent[y as usize];
    if parent[y as usize] == NOSTREAM {
        *root = x;
    }
    else if y == right[parent[y as usize] as usize] {
        right[parent[y as usize] as usize] = x;
    }
    else {
        left[parent[y as usize] as usize] = x;
    }
    right[x as usize] = y;
    parent[y as usize] = x;
}

#[allow(clippy::too_many_arguments)]
fn write_entry(dir: &mut [u8], id: usize, name: &str, kind: u8, child: u32, start: u32, size: u64, left: u32, right: u32, color: u8) {
    let at = id * ENTRY;
    let units: Vec<u16> = name.encode_utf16().collect();
    let trimmed = units.len().min(31);
    for (index, unit) in units[..trimmed].iter().enumerate() {
        dir[at + index * 2..at + index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    dir[at + 0x40..at + 0x42].copy_from_slice(&(((trimmed + 1) * 2) as u16).to_le_bytes());
    dir[at + 0x42] = kind;
    dir[at + 0x43] = color;
    dir[at + 0x44..at + 0x48].copy_from_slice(&left.to_le_bytes());
    dir[at + 0x48..at + 0x4C].copy_from_slice(&right.to_le_bytes());
    dir[at + 0x4C..at + 0x50].copy_from_slice(&child.to_le_bytes());
    dir[at + 0x74..at + 0x78].copy_from_slice(&start.to_le_bytes());
    dir[at + 0x78..at + 0x7C].copy_from_slice(&(size as u32).to_le_bytes());
    dir[at + 0x7C..at + 0x80].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
}

fn write_header(file: &mut [u8], fat_sectors: u32, dir_start: u32, minifat_start: u32, minifat_sectors: u32, fat_start: u32, dir_sectors: u32) {
    file[0..8].copy_from_slice(&SIGNATURE);
    file[0x18..0x1A].copy_from_slice(&0x003Eu16.to_le_bytes());
    file[0x1A..0x1C].copy_from_slice(&0x0004u16.to_le_bytes());
    file[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
    file[0x1E..0x20].copy_from_slice(&(SECTOR.trailing_zeros() as u16).to_le_bytes());
    file[0x20..0x22].copy_from_slice(&(MINI.trailing_zeros() as u16).to_le_bytes());
    // v4 专用：目录扇区数（v3 必须为 0，v4 必须为实际值，否则 ole32 认为目录为空）。
    file[0x28..0x2C].copy_from_slice(&dir_sectors.to_le_bytes());
    file[0x2C..0x30].copy_from_slice(&fat_sectors.to_le_bytes());
    file[0x30..0x34].copy_from_slice(&dir_start.to_le_bytes());
    file[0x38..0x3C].copy_from_slice(&(MAX_MINI as u32).to_le_bytes());
    file[0x3C..0x40].copy_from_slice(&minifat_start.to_le_bytes());
    file[0x40..0x44].copy_from_slice(&minifat_sectors.to_le_bytes());
    file[0x44..0x48].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
    file[0x48..0x4C].copy_from_slice(&0u32.to_le_bytes());
    for index in 0..DIFAT_INLINE {
        let value = if (index as u32) < fat_sectors { fat_start + index as u32 } else { FREESECT };
        file[0x4C + index * 4..0x50 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// 回读器：解析目录并返回 `path → bytes`（存储内流路径为 `存储/流`）。
/// 支持 v3/v4、链式 DIFAT 与真实红黑树目录（中序遍历收集）。
pub(crate) fn read_streams(bytes: &[u8]) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    use std::collections::BTreeMap;

    if bytes.len() < SECTOR || bytes[0..8] != SIGNATURE {
        return Err(InstallerError::Verify("bad CFB signature".into()));
    }
    let header_u32 = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let sector_size = 1usize << u16::from_le_bytes(bytes[0x1E..0x20].try_into().unwrap());
    let mini_size = 1usize << u16::from_le_bytes(bytes[0x20..0x22].try_into().unwrap());
    let major = u16::from_le_bytes(bytes[0x1A..0x1C].try_into().unwrap());
    let max_mini = header_u32(0x38) as usize;
    let dir_start = header_u32(0x30);
    let minifat_start = header_u32(0x3C);
    let difat_start = header_u32(0x44);
    let difat_sectors = header_u32(0x48) as usize;
    let sector = |id: u32| &bytes[sector_size * (1 + id as usize)..sector_size * (2 + id as usize)];
    let chain = |table: &[u32], mut id: u32| -> Vec<u32> {
        let mut out = Vec::new();
        while id != ENDOFCHAIN && id != FREESECT && (id as usize) < table.len() {
            out.push(id);
            id = table[id as usize];
        }
        out
    };

    // DIFAT：内联 109 项 + 链式扇区。
    let mut fat_ids: Vec<u32> = Vec::new();
    for index in 0..DIFAT_INLINE {
        let value = header_u32(0x4C + index * 4);
        if value != FREESECT {
            fat_ids.push(value);
        }
    }
    let mut difat_chain = difat_start;
    for _ in 0..difat_sectors {
        if difat_chain == ENDOFCHAIN {
            break;
        }
        let block = sector(difat_chain);
        for at in (0..block.len() - 4).step_by(4) {
            let value = u32::from_le_bytes(block[at..at + 4].try_into().unwrap());
            if value != FREESECT {
                fat_ids.push(value);
            }
        }
        difat_chain = u32::from_le_bytes(block[block.len() - 4..].try_into().unwrap());
    }
    let mut fat: Vec<u32> = Vec::with_capacity(fat_ids.len() * (sector_size / 4));
    for id in fat_ids {
        let data = sector(id);
        for at in (0..data.len()).step_by(4) {
            fat.push(u32::from_le_bytes(data[at..at + 4].try_into().unwrap()));
        }
    }
    let mut minifat: Vec<u32> = Vec::new();
    for id in chain(&fat, minifat_start) {
        let data = sector(id);
        for at in (0..data.len()).step_by(4) {
            minifat.push(u32::from_le_bytes(data[at..at + 4].try_into().unwrap()));
        }
    }
    // root 目录项在 dir_start 扇区偏移 0；root.start 在项内 0x74，即 mini 流数据链首。
    let root_start = u32::from_le_bytes(sector(dir_start)[0x74..0x78].try_into().unwrap());
    let root_sectors = chain(&fat, root_start);
    let mini_data = |start: u32, size: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(size);
        for id in chain(&minifat, start) {
            let sector_index = id as usize / (sector_size / mini_size);
            let offset = (id as usize % (sector_size / mini_size)) * mini_size;
            let at = sector_size * (1 + root_sectors[sector_index] as usize) + offset;
            out.extend_from_slice(&bytes[at..at + mini_size]);
        }
        out.truncate(size);
        out
    };
    let regular = |start: u32, size: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(size);
        for id in chain(&fat, start) {
            out.extend_from_slice(sector(id));
        }
        out.truncate(size);
        out
    };

    let mut entries: Vec<(String, u8, u32, u32, u32, u32, u64)> = Vec::new();
    for id in chain(&fat, dir_start) {
        let block = sector(id);
        for item_index in 0..block.len() / ENTRY {
            let item = &block[item_index * ENTRY..item_index * ENTRY + ENTRY];
            let kind = item[0x42];
            let name_len = u16::from_le_bytes(item[0x40..0x42].try_into().unwrap()) as usize;
            if kind == TYPE_UNKNOWN || name_len < 2 {
                entries.push((String::new(), TYPE_UNKNOWN, NOSTREAM, NOSTREAM, NOSTREAM, NOSTREAM, 0));
                continue;
            }
            let mut units: Vec<u16> = Vec::with_capacity(name_len / 2);
            for at in (0..name_len - 2).step_by(2) {
                units.push(u16::from_le_bytes(item[at..at + 2].try_into().unwrap()));
            }
            let child = u32::from_le_bytes(item[0x4C..0x50].try_into().unwrap());
            let left = u32::from_le_bytes(item[0x44..0x48].try_into().unwrap());
            let right = u32::from_le_bytes(item[0x48..0x4C].try_into().unwrap());
            let start = u32::from_le_bytes(item[0x74..0x78].try_into().unwrap());
            let size = if major >= 4 {
                u64::from_le_bytes(item[0x78..0x80].try_into().unwrap())
            }
            else {
                u32::from_le_bytes(item[0x78..0x7C].try_into().unwrap()) as u64
            };
            entries.push((String::from_utf16_lossy(&units), kind, left, right, child, start, size));
        }
    }
    fn walk(entries: &[(String, u8, u32, u32, u32, u32, u64)], id: u32, out: &mut Vec<u32>) {
        if id == NOSTREAM || id as usize >= entries.len() {
            return;
        }
        let (_, _, left, right, _, _, _) = &entries[id as usize];
        walk(entries, *left, out);
        out.push(id);
        walk(entries, *right, out);
    }
    let collect = |id: u32| -> Vec<u32> {
        let mut out = Vec::new();
        walk(&entries, id, &mut out);
        out
    };
    let read_one = |start: u32, size: u64| -> Vec<u8> {
        if size == 0 {
            Vec::new()
        }
        else if (size as usize) < max_mini {
            mini_data(start, size as usize)
        }
        else {
            regular(start, size as usize)
        }
    };

    let mut result = BTreeMap::new();
    let root_child = entries[0].4;
    for id in collect(root_child) {
        let (name, kind, _, _, child, start, size) = &entries[id as usize];
        match *kind {
            TYPE_STREAM => {
                result.insert(name.clone(), read_one(*start, *size));
            }
            TYPE_STORAGE => {
                for child_id in collect(*child) {
                    let (cname, ckind, _, _, _, cstart, csize) = &entries[child_id as usize];
                    if *ckind == TYPE_STREAM {
                        result.insert(format!("{name}/{cname}"), read_one(*cstart, *csize));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 诊断（Windows，ignored）：dump 真实 MSI 与本 crate 产物的目录树
    /// （id/name/type/color/left/right/child），用于对照 ole32 期望的形状。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn dump_directory_trees() {
        fn dump(label: &str, bytes: &[u8]) {
            let sector_size = 1usize << u16::from_le_bytes(bytes[0x1E..0x20].try_into().unwrap());
            let dir_start = u32::from_le_bytes(bytes[0x30..0x34].try_into().unwrap());
            let difat_start = u32::from_le_bytes(bytes[0x44..0x48].try_into().unwrap());
            let difat_count = u32::from_le_bytes(bytes[0x48..0x4C].try_into().unwrap()) as usize;
            let sector = |id: u32| &bytes[sector_size * (1 + id as usize)..sector_size * (2 + id as usize)];
            let mut fat_ids: Vec<u32> = (0..DIFAT_INLINE)
                .map(|i| u32::from_le_bytes(bytes[0x4C + i * 4..0x50 + i * 4].try_into().unwrap()))
                .filter(|v| *v != FREESECT)
                .collect();
            let mut next = difat_start;
            for _ in 0..difat_count {
                if next == ENDOFCHAIN {
                    break;
                }
                let block = sector(next);
                for at in (0..block.len() - 4).step_by(4) {
                    let v = u32::from_le_bytes(block[at..at + 4].try_into().unwrap());
                    if v != FREESECT {
                        fat_ids.push(v);
                    }
                }
                next = u32::from_le_bytes(block[block.len() - 4..].try_into().unwrap());
            }
            let mut fat: Vec<u32> = Vec::new();
            for id in &fat_ids {
                let d = sector(*id);
                for at in (0..d.len()).step_by(4) {
                    fat.push(u32::from_le_bytes(d[at..at + 4].try_into().unwrap()));
                }
            }
            let chain = |mut id: u32| -> Vec<u32> {
                let mut out = Vec::new();
                while id != ENDOFCHAIN && id != FREESECT && (id as usize) < fat.len() {
                    out.push(id);
                    id = fat[id as usize];
                }
                out
            };
            let mut entries: Vec<(String, u8, u8, u32, u32, u32)> = Vec::new();
            for sid in chain(dir_start) {
                let block = sector(sid);
                for i in 0..block.len() / ENTRY {
                    let item = &block[i * ENTRY..i * ENTRY + ENTRY];
                    let kind = item[0x42];
                    let color = item[0x43];
                    let name_len = u16::from_le_bytes(item[0x40..0x42].try_into().unwrap()) as usize;
                    if kind == TYPE_UNKNOWN || name_len < 2 {
                        entries.push((String::new(), TYPE_UNKNOWN, 0, NOSTREAM, NOSTREAM, NOSTREAM));
                        continue;
                    }
                    let units: Vec<u16> =
                        (0..name_len - 2).step_by(2).map(|at| u16::from_le_bytes(item[at..at + 2].try_into().unwrap())).collect();
                    let left = u32::from_le_bytes(item[0x44..0x48].try_into().unwrap());
                    let right = u32::from_le_bytes(item[0x48..0x4C].try_into().unwrap());
                    let child = u32::from_le_bytes(item[0x4C..0x50].try_into().unwrap());
                    entries.push((String::from_utf16_lossy(&units), kind, color, left, right, child));
                }
            }
            println!("== {label}: sectorSize={sector_size} entries={} ==", entries.len());
            let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
            println!(
                "  hdr: minor={minor:X} major={major:X} sectorShift={ss:X} miniShift={ms:X} numDirSec={num_dir} numFat={num_fat} dirStart={dir_start} txSig={tx} cutoff={cutoff} minifatStart={mf} numMinifat={nmf} difatStart={ds} numDifat={nd}",
                minor = u16_at(0x18),
                major = u16_at(0x1A),
                ss = u16_at(0x1E),
                ms = u16_at(0x20),
                num_dir = u32_at(0x28),
                num_fat = u32_at(0x2C),
                dir_start = u32_at(0x30),
                tx = u32_at(0x34),
                cutoff = u32_at(0x38),
                mf = u32_at(0x3C),
                nmf = u32_at(0x40),
                ds = u32_at(0x44),
                nd = u32_at(0x48)
            );
            for sid in chain(dir_start).iter().take(1) {
                let block = sector(*sid);
                for i in 0..block.len() / ENTRY {
                    let item = &block[i * ENTRY..i * ENTRY + ENTRY];
                    if item[0x42] == TYPE_UNKNOWN {
                        continue;
                    }
                    let nl = u16::from_le_bytes(item[0x40..0x42].try_into().unwrap());
                    let hex: String = item[0..(nl as usize).min(0x40)].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
                    println!("  raw[{i}]: len={nl} {hex}");
                }
            }
            for (id, (name, kind, color, left, right, child)) in entries.iter().enumerate() {
                if *kind == TYPE_UNKNOWN {
                    continue;
                }
                println!("  [{id}] {name:?} type={kind} color={color} left={left} right={right} child={child}");
            }
            fn inorder(entries: &[(String, u8, u8, u32, u32, u32)], id: u32, out: &mut Vec<String>) {
                if id == NOSTREAM || id as usize >= entries.len() {
                    return;
                }
                let (name, _, _, left, right, _) = &entries[id as usize];
                inorder(entries, *left, out);
                out.push(name.clone());
                inorder(entries, *right, out);
            }
            let mut order = Vec::new();
            inorder(&entries, entries[0].5, &mut order);
            println!("  root-child in-order: {order:?}");
        }

        let dir = std::path::Path::new(r"C:\Windows\Installer");
        let reals: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("msi")
                    && std::fs::metadata(p).is_ok_and(|m| m.len() > 4096 && m.len() < 4_000_000)
            })
            .take(4)
            .collect();
        assert!(!reals.is_empty(), "no readable .msi under C:\\Windows\\Installer");
        for real in &reals {
            dump("real", &std::fs::read(real).unwrap());
        }

        let mut cfb = Cfb::new();
        for name in ["_Tables", "_Columns", "_StringPool", "_StringData", "Property", "FileData", "\u{1}SummaryInformation"] {
            cfb.add_stream(name, vec![0x5Au8; if name == "FileData" { 5000 } else { 100 }]).unwrap();
        }
        cfb.add_stream("##DataSpaces/Version", vec![0x5Au8; 24]).unwrap();
        dump("ours", &cfb.finish().unwrap());
    }

    /// 诊断（Windows，ignored）：按大小 dump 真实 MSI 各流前 40 字节，
    /// 用于钉死 `_StringPool` / `_StringData` / `_Tables` 的真实字节格式。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn dump_real_stream_bytes() {
        let dir = std::path::Path::new(r"C:\Windows\Installer");
        let real = std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .find(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("msi")
                    && std::fs::metadata(p).is_ok_and(|m| m.len() > 4096 && m.len() < 2_000_000)
            })
            .expect("no real msi");
        let bytes = std::fs::read(&real).unwrap();
        let streams = read_streams(&bytes).unwrap();
        let mut by_size: Vec<(usize, String, Vec<u8>)> = streams.iter().map(|(k, v)| (v.len(), k.clone(), v.clone())).collect();
        by_size.sort();
        for (size, name, data) in &by_size {
            let head: String = data.iter().take(40).map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
            let ascii: String = data.iter().take(40).map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' }).collect();
            println!("size={size:6} name={name:?}\n    hex={head}\n    asc={ascii}");
        }
    }

    #[test]
    fn rb_tree_is_valid() {
        let mut cfb = Cfb::new();
        let names: Vec<String> = (0..13).map(|i| format!("stream_{i:02}_pad")).collect();
        for name in &names {
            cfb.add_stream(name, vec![0u8; 10]).unwrap();
        }
        let bytes = cfb.finish().unwrap();
        let dir_start = u32::from_le_bytes(bytes[0x30..0x34].try_into().unwrap());
        let block = &bytes[SECTOR * (1 + dir_start as usize)..SECTOR * (2 + dir_start as usize)];
        let mut entries: Vec<(String, u32, u32, u32, u8)> = Vec::new();
        for i in 0..block.len() / ENTRY {
            let item = &block[i * ENTRY..i * ENTRY + ENTRY];
            let nl = u16::from_le_bytes(item[0x40..0x42].try_into().unwrap()) as usize;
            if item[0x42] == TYPE_UNKNOWN || nl < 2 {
                entries.push((String::new(), NOSTREAM, NOSTREAM, NOSTREAM, 1));
                continue;
            }
            let units: Vec<u16> = (0..nl - 2).step_by(2).map(|at| u16::from_le_bytes(item[at..at + 2].try_into().unwrap())).collect();
            let left = u32::from_le_bytes(item[0x44..0x48].try_into().unwrap());
            let right = u32::from_le_bytes(item[0x48..0x4C].try_into().unwrap());
            let child = u32::from_le_bytes(item[0x4C..0x50].try_into().unwrap());
            let color = item[0x43];
            entries.push((String::from_utf16_lossy(&units), left, right, child, color));
        }
        let mut inorder: Vec<String> = Vec::new();
        fn walk(entries: &[(String, u32, u32, u32, u8)], id: u32, out: &mut Vec<String>) {
            if id == NOSTREAM {
                return;
            }
            let (name, left, right, _, _) = &entries[id as usize];
            walk(entries, *left, out);
            out.push(name.clone());
            walk(entries, *right, out);
        }
        walk(&entries, entries[0].3, &mut inorder);
        assert_eq!(inorder.len(), names.len(), "not all children reachable");
        for pair in inorder.windows(2) {
            assert_eq!(name_cmp(&pair[0], &pair[1]), std::cmp::Ordering::Less, "BST order violated: {:?} !< {:?}", pair[0], pair[1]);
        }
        fn check_rb(entries: &[(String, u32, u32, u32, u8)], id: u32, black: u32, expect: &mut Option<u32>, prev_red: bool) {
            if id == NOSTREAM {
                if let Some(e) = *expect {
                    assert_eq!(e, black, "black-height mismatch");
                }
                else {
                    *expect = Some(black);
                }
                return;
            }
            let (_, left, right, _, color) = &entries[id as usize];
            let is_red = *color == RED;
            assert!(!(is_red && prev_red), "red-red violation at {}", entries[id as usize].0);
            let nb = black + if is_red { 0 } else { 1 };
            check_rb(entries, *left, nb, expect, is_red);
            check_rb(entries, *right, nb, expect, is_red);
        }
        let root = entries[0].3;
        assert_eq!(entries[root as usize].4, BLACK, "tree root must be black");
        let mut expect = None;
        check_rb(&entries, root, 0, &mut expect, false);
    }

    #[test]
    fn round_trips_mini_regular_and_nested() {
        let mut cfb = Cfb::new();
        cfb.add_stream("_Tables", b"Property\r\nDirectory\r\n".to_vec()).unwrap();
        cfb.add_stream("##DataSpaces/Version", vec![7u8; 24]).unwrap();
        cfb.add_stream("##DataSpaces/Validation", vec![9u8; 12]).unwrap();
        cfb.add_stream("FileData", vec![0xABu8; 5000]).unwrap();
        let bytes = cfb.finish().unwrap();
        assert_eq!(&bytes[0..8], &SIGNATURE);
        let streams = read_streams(&bytes).unwrap();
        assert_eq!(streams["_Tables"], b"Property\r\nDirectory\r\n");
        assert_eq!(streams["##DataSpaces/Version"], vec![7u8; 24]);
        assert_eq!(streams["##DataSpaces/Validation"], vec![9u8; 12]);
        assert_eq!(streams["FileData"], vec![0xABu8; 5000]);
    }

    #[test]
    fn boundary_4095_mini_4096_regular() {
        let mut cfb = Cfb::new();
        cfb.add_stream("mini", vec![0x5A; MAX_MINI - 1]).unwrap();
        cfb.add_stream("regular", vec![0xA5; MAX_MINI]).unwrap();
        let streams = read_streams(&cfb.finish().unwrap()).unwrap();
        assert_eq!(streams["mini"], vec![0x5A; MAX_MINI - 1]);
        assert_eq!(streams["regular"], vec![0xA5; MAX_MINI]);
    }

    #[test]
    fn empty_stream_and_only_root() {
        let mut cfb = Cfb::new();
        cfb.add_stream("empty", Vec::new()).unwrap();
        let streams = read_streams(&cfb.finish().unwrap()).unwrap();
        assert_eq!(streams["empty"], Vec::<u8>::new());

        let empty = Cfb::new();
        let bytes = empty.finish().unwrap();
        assert_eq!(&bytes[0..8], &SIGNATURE);
        assert!(read_streams(&bytes).unwrap().is_empty());
    }

    #[test]
    fn many_streams_fill_multiple_dir_sectors() {
        let mut cfb = Cfb::new();
        for index in 0..10 {
            cfb.add_stream(&format!("s{index}"), vec![index as u8; 60]).unwrap();
        }
        let streams = read_streams(&cfb.finish().unwrap()).unwrap();
        assert_eq!(streams.len(), 10);
        for index in 0..10 {
            assert_eq!(streams[&format!("s{index}")], vec![index as u8; 60]);
        }
    }

    #[test]
    fn rejects_duplicate_and_empty_segments() {
        let mut cfb = Cfb::new();
        cfb.add_stream("a", Vec::new()).unwrap();
        assert!(cfb.add_stream("a", Vec::new()).is_err());
        assert!(cfb.add_stream("a//b", Vec::new()).is_err());
        assert!(cfb.add_stream("/a", Vec::new()).is_err());
    }

    #[test]
    fn multi_mini_streams_chain_independently() {
        let mut cfb = Cfb::new();
        cfb.add_stream("one", vec![1u8; 100]).unwrap();
        cfb.add_stream("two", vec![2u8; 3000]).unwrap();
        cfb.add_stream("three", vec![3u8; 64]).unwrap();
        let streams = read_streams(&cfb.finish().unwrap()).unwrap();
        assert_eq!(streams["one"], vec![1u8; 100]);
        assert_eq!(streams["two"], vec![2u8; 3000]);
        assert_eq!(streams["three"], vec![3u8; 64]);
    }

    /// Schema 探针（Windows）：解析本机真实 MSI 缓存包并转储关键流的字节，
    /// 用于把 `_StringPool` / `_StringData` / 行编码 / `##DataSpaces` /
    /// `_SummaryInformation` 的格式钉死为代码常量。`cargo test -- --nocapture`
    /// 查看输出；没有可读 `.msi` 时跳过。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn probe_real_msi_streams() {
        let dir = std::path::Path::new(r"C:\Windows\Installer");
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("msi")
                    && std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 1024 && meta.len() < 40_000_000)
                {
                    candidates.push(path);
                    if candidates.len() == 5 {
                        break;
                    }
                }
            }
        }
        if candidates.is_empty() {
            eprintln!("probe: no readable .msi under {dir:?}; skipped");
            return;
        }
        // 多文件对照：`_Tables` UTF-16LE 是否存在、目录首项是否 "Root Entry"。
        let needle: Vec<u8> = "_Tables".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let root_needle: Vec<u8> = "Root Entry".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        for path in &candidates {
            let Ok(bytes) = std::fs::read(path)
            else {
                println!("{}: unreadable", path.display());
                continue;
            };
            if bytes.len() < 512 || bytes[0..8] != SIGNATURE {
                println!("{}: not CFB", path.display());
                continue;
            }
            let sector_shift = u16::from_le_bytes(bytes[0x1E..0x20].try_into().unwrap());
            let dir_start = u32::from_le_bytes(bytes[0x30..0x34].try_into().unwrap());
            let at = (1usize + dir_start as usize) << sector_shift;
            let root_ok = at + root_needle.len() <= bytes.len() && bytes[at..at + root_needle.len()] == root_needle[..];
            let tables_at = (0..bytes.len().saturating_sub(needle.len())).find(|i| bytes[*i..*i + needle.len()] == needle[..]);
            println!(
                "{}: size={} sectorShift={sector_shift} dirStart={dir_start} rootAtDirStart={root_ok} _TablesUtf16At={tables_at:?}",
                path.display(),
                bytes.len()
            );
        }
        let path = candidates[0].clone();
        for (label, query) in [
            ("COLUMNS", "SELECT * FROM `_Columns`"),
            ("PROPERTY", "SELECT * FROM `Property`"),
            ("SEQUENCE", "SELECT * FROM `InstallExecuteSequence`"),
            ("MEDIA", "SELECT * FROM `Media`"),
            ("DIRECTORY", "SELECT * FROM `Directory`"),
            ("COMPONENT", "SELECT * FROM `Component`"),
            ("FILE", "SELECT * FROM `File`"),
            ("FEATURE", "SELECT * FROM `Feature`"),
        ] {
            match crate::verify::msi_api_query(&path, query) {
                Ok(rows) => {
                    println!("== {label} ({} rows) ==", rows.len());
                    for row in rows.into_iter().take(60) {
                        println!("  {}", row.join("\t"));
                    }
                }
                Err(error) => println!("== {label} == {error}"),
            }
        }
        let bytes = std::fs::read(&path).expect("read candidate");
        println!("probe: {}", path.display());
        // 自包含 header 解析。
        let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let sector_size = 1usize << u16_at(0x1E);
        let mini_size = 1usize << u16_at(0x20);
        let max_mini = u32_at(0x38) as usize;
        let dir_start = u32_at(0x30);
        let minifat_start = u32_at(0x3C);
        let difat_start = u32_at(0x44);
        let difat_count = u32_at(0x48) as usize;
        let sector = |id: u32| &bytes[sector_size * (1 + id as usize)..sector_size * (2 + id as usize)];
        let mut fat_ids: Vec<u32> = (0..DIFAT_INLINE).map(|i| u32_at(0x4C + i * 4)).filter(|v| *v != FREESECT).collect();
        let mut next = difat_start;
        for _ in 0..difat_count {
            if next == ENDOFCHAIN {
                break;
            }
            let b = sector(next);
            for at in (0..b.len() - 4).step_by(4) {
                let v = u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
                if v != FREESECT {
                    fat_ids.push(v);
                }
            }
            next = u32::from_le_bytes(b[b.len() - 4..].try_into().unwrap());
        }
        let mut fat: Vec<u32> = Vec::new();
        for id in &fat_ids {
            let d = sector(*id);
            for at in (0..d.len()).step_by(4) {
                fat.push(u32::from_le_bytes(d[at..at + 4].try_into().unwrap()));
            }
        }
        let chain = |table: &[u32], mut id: u32| -> Vec<u32> {
            let mut out = Vec::new();
            while id != ENDOFCHAIN && id != FREESECT && (id as usize) < table.len() {
                out.push(id);
                id = table[id as usize];
            }
            out
        };
        let mut minifat: Vec<u32> = Vec::new();
        for id in chain(&fat, minifat_start) {
            let d = sector(id);
            for at in (0..d.len()).step_by(4) {
                minifat.push(u32::from_le_bytes(d[at..at + 4].try_into().unwrap()));
            }
        }
        let root_start = u32::from_le_bytes(sector(dir_start)[0x74..0x78].try_into().unwrap());
        let root_sectors = chain(&fat, root_start);
        let read_stream = |start: u32, size: u64| -> Vec<u8> {
            let size = size as usize;
            if size == 0 {
                return Vec::new();
            }
            if size < max_mini {
                let mut out = Vec::with_capacity(size);
                for id in chain(&minifat, start) {
                    let si = id as usize / (sector_size / mini_size);
                    let off = (id as usize % (sector_size / mini_size)) * mini_size;
                    out.extend_from_slice(&bytes[sector_size * (1 + root_sectors[si] as usize) + off..][..mini_size]);
                }
                out.truncate(size);
                out
            }
            else {
                let mut out = Vec::with_capacity(size);
                for id in chain(&fat, start) {
                    out.extend_from_slice(sector(id));
                }
                out.truncate(size);
                out
            }
        };
        // 目录解码：尝试 name 在 entry+2（len 含前导 NUL）。
        let mut entries: Vec<(String, u8, u32, u32, u32, u32, u64)> = Vec::new();
        for sid in chain(&fat, dir_start) {
            let block = sector(sid);
            for i in 0..block.len() / ENTRY {
                let item = &block[i * ENTRY..i * ENTRY + ENTRY];
                let kind = item[0x42];
                let name_len = u16::from_le_bytes(item[0x40..0x42].try_into().unwrap()) as usize;
                if kind == TYPE_UNKNOWN || name_len < 4 {
                    entries.push((String::new(), TYPE_UNKNOWN, NOSTREAM, NOSTREAM, NOSTREAM, NOSTREAM, 0));
                    continue;
                }
                let mut units: Vec<u16> = Vec::new();
                for at in (2..name_len - 2).step_by(2) {
                    units.push(u16::from_le_bytes(item[at..at + 2].try_into().unwrap()));
                }
                let child = u32::from_le_bytes(item[0x4C..0x50].try_into().unwrap());
                let left = u32::from_le_bytes(item[0x44..0x48].try_into().unwrap());
                let right = u32::from_le_bytes(item[0x48..0x4C].try_into().unwrap());
                let start = u32::from_le_bytes(item[0x74..0x78].try_into().unwrap());
                let size = u32::from_le_bytes(item[0x78..0x7C].try_into().unwrap()) as u64;
                entries.push((String::from_utf16_lossy(&units), kind, left, right, child, start, size));
            }
        }
        fn walk(entries: &[(String, u8, u32, u32, u32, u32, u64)], id: u32, out: &mut Vec<u32>) {
            if id == NOSTREAM || id as usize >= entries.len() {
                return;
            }
            let (_, _, left, right, _, _, _) = &entries[id as usize];
            walk(entries, *left, out);
            out.push(id);
            walk(entries, *right, out);
        }
        let collect = |id: u32| -> Vec<u32> {
            let mut out = Vec::new();
            walk(&entries, id, &mut out);
            out
        };
        let mut by_path = std::collections::BTreeMap::new();
        for id in collect(entries[0].4) {
            let (name, kind, _, _, child, start, size) = &entries[id as usize];
            match *kind {
                TYPE_STREAM => {
                    by_path.insert(name.clone(), (*start, *size));
                }
                TYPE_STORAGE => {
                    for cid in collect(*child) {
                        let (cn, ck, _, _, _, cs, csize) = &entries[cid as usize];
                        if *ck == TYPE_STREAM {
                            by_path.insert(format!("{name}/{cn}"), (*cs, *csize));
                        }
                    }
                }
                _ => {}
            }
        }
        println!("dir names: {:?}", by_path.keys().collect::<Vec<_>>());
        // 转储所有 ≤150 字节的流（DataSpaces/_StringData/_Tables 候选）。
        for (name, (start, size)) in &by_path {
            if *size <= 150 {
                let data = read_stream(*start, *size);
                println!("stream {name} ({}): {}", size, hexline(&data[..data.len().min(96)]));
            }
        }
        // 9558/1704 候选池与表流的头部。
        for (name, (start, size)) in &by_path {
            if *size == 9558 || *size == 1704 || *size == 660 {
                let data = read_stream(*start, *size);
                println!("head {name} ({}): {}", size, hexline(&data[..data.len().min(128)]));
            }
        }
        let pool = by_path.get("_StringPool").map(|(s, z)| read_stream(*s, *z)).unwrap_or_default();
        let pool_at = |block: u32, index: u32| -> String {
            let mut count = 0u32;
            let mut at = block as usize;
            while at + 4 < pool.len() {
                let block_size = u32::from_le_bytes(pool[at..at + 4].try_into().unwrap()) as usize;
                let mut cursor = at + 8;
                let block_end = at + block_size.max(8);
                while cursor < block_end && cursor < pool.len() {
                    let end = pool[cursor..].iter().position(|b| *b == 0).map(|p| cursor + p).unwrap_or(block_end);
                    if count == index {
                        return String::from_utf8_lossy(&pool[cursor..end]).into_owned();
                    }
                    count += 1;
                    cursor = end + 1;
                }
                if block_size == 0 {
                    break;
                }
                at += block_size;
            }
            String::new()
        };
        let parse_rows = |data: &[u8]| -> Vec<Vec<String>> {
            let mut rows = Vec::new();
            let mut at = 0usize;
            while at + 4 <= data.len() {
                let row_size = u16::from_le_bytes(data[at..at + 2].try_into().unwrap()) as usize;
                if row_size == 0 {
                    break;
                }
                let fields = u16::from_le_bytes(data[at + 2..at + 4].try_into().unwrap()) as usize;
                let mut row = Vec::with_capacity(fields);
                let mut widths = Vec::with_capacity(fields);
                for f in 0..fields {
                    widths.push(u16::from_le_bytes(data[at + 4 + f * 2..at + 6 + f * 2].try_into().unwrap()));
                }
                let mut cursor = at + 4 + fields * 2;
                for width in widths {
                    if width == 0xFFFF {
                        row.push(String::new());
                    }
                    else if width == 12 {
                        let stream = u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap());
                        let block = u32::from_le_bytes(data[cursor + 4..cursor + 8].try_into().unwrap());
                        let index = u32::from_le_bytes(data[cursor + 8..cursor + 12].try_into().unwrap());
                        row.push(format!("{stream}:{block}:{index}={}", pool_at(block, index)));
                        cursor += 12;
                    }
                    else if width == 4 {
                        row.push(u32::from_le_bytes(data[cursor..cursor + 4].try_into().unwrap()).to_string());
                        cursor += 4;
                    }
                    else {
                        cursor += width as usize;
                        row.push("?".into());
                    }
                }
                rows.push(row);
                at += row_size;
            }
            rows
        };
        for name in ["_Columns", "_Tables", "Property", "File", "Media", "Directory", "Component", "Feature"] {
            if let Some((start, size)) = by_path.get(name) {
                let data = read_stream(*start, *size);
                println!("== {name} ({} bytes) ==", data.len());
                for row in parse_rows(&data).into_iter().take(40) {
                    println!("  {row:?}");
                }
            }
            else {
                println!("== {name} == MISSING");
            }
        }
        for path in ["##DataSpaces/Validation", "##DataSpaces/Version"] {
            if let Some((start, size)) = by_path.get(path) {
                let data = read_stream(*start, *size);
                println!("{path}: {}", hexline(&data[..data.len().min(48)]));
            }
        }
        if let Some((start, size)) = by_path.get("\u{1}SummaryInformation").or_else(|| by_path.get("SummaryInformation")) {
            let data = read_stream(*start, *size);
            println!("SummaryInformation: {}", hexline(&data[..data.len().min(96)]));
        }
    }

    #[cfg(windows)]
    fn hexdump(data: &[u8], limit: usize) {
        for (row, chunk) in data.chunks(16).take(limit / 16).enumerate() {
            let hex: Vec<String> = chunk.iter().map(|byte| format!("{byte:02X}")).collect();
            let ascii: String = chunk.iter().map(|byte| if (0x20..0x7F).contains(byte) { *byte as char } else { '.' }).collect();
            println!("{:08X}  {:<48}  {ascii}", row * 16, hex.join(" "));
        }
    }

    #[cfg(windows)]
    fn hexline(data: &[u8]) -> String {
        data.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" ")
    }
}
