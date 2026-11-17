//! CFB v3（MS-CFB / OLE 复合文档）writer：512 字节扇区、64 字节 mini 扇区、
//! miniFAT 与内联 DIFAT（≤109 个 FAT 扇区，产物上限约 3.5 GB）。
//!
//! MSI 数据库就是这个容器：`_Tables`、`_StringPool` 等小流（<4096 字节）
//! 进 mini 扇区，内嵌 cabinet 大流走常规扇区链。目录树按仓内既有工具
//! （msitools 同款）允许的方式退化为「child 指向首项、right 串兄弟」的
//! 线性形状，颜色全黑；读取方中序遍历即可收集全部子项，不依赖平衡性。
//!
//! v0 在内存中组装完整文件字节（构建期工具可接受）。`add_stream` 的
//! `path` 用 `/` 分隔存储层级，缺失的中间段自动成为存储。

use std::io::Write;

use crate::error::{InstallerError, Result};

/// 常规扇区大小（字节）。
const SECTOR: usize = 512;
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
            return Err(InstallerError::Layout(format!("CFB needs {fat_sectors} FAT sectors; inline DIFAT caps at {DIFAT_INLINE} (~3.5 GB)")));
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

        // 6. 目录项。
        let mut dir = vec![0u8; dir_sectors * SECTOR];
        // Root：start = mini 流链首，size = mini 流总字节。
        let root_start = if mini_stream_sectors > 0 { mini_stream_start } else { ENDOFCHAIN };
        write_entry(&mut dir, 0, "Root Entry", TYPE_ROOT, first_child(&self.nodes[0]), root_start, mini_stream_bytes as u64);
        for (id, node) in self.nodes.iter().enumerate().skip(1) {
            let (kind, start, size) = match &node.kind {
                Kind::Storage => (TYPE_STORAGE, 0u32, 0u64),
                Kind::Stream(data) => match placements[id] {
                    Placement::None => (TYPE_STREAM, ENDOFCHAIN, 0),
                    Placement::Mini { start, .. } => (TYPE_STREAM, start, data.len() as u64),
                    Placement::Regular { start, .. } => (TYPE_STREAM, start, data.len() as u64),
                },
                Kind::Root => unreachable!("root handled above"),
            };
            write_entry(&mut dir, id, &node.name, kind, first_child(node), start, size);
        }
        // 兄弟链：同父子项按插入顺序串 right。
        for node in &self.nodes {
            for (index, child_id) in node.children.iter().enumerate() {
                let at = child_id * ENTRY;
                let right = node.children.get(index + 1).copied().unwrap_or(NOSTREAM as usize);
                dir[at + 0x48..at + 0x4C].copy_from_slice(&(right as u32).to_le_bytes());
            }
        }

        // 7. 组装文件。
        let mut file = vec![0u8; SECTOR * (1 + total_sectors)];
        let minifat_header = if minifat_sectors > 0 { minifat_start } else { ENDOFCHAIN };
        write_header(&mut file, fat_sectors as u32, dir_start, minifat_header, minifat_sectors as u32, fat_start);
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

fn first_child(node: &Node) -> u32 {
    node.children.first().copied().map_or(NOSTREAM, |id| id as u32)
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

fn write_entry(dir: &mut [u8], id: usize, name: &str, kind: u8, child: u32, start: u32, size: u64) {
    let at = id * ENTRY;
    let units: Vec<u16> = name.encode_utf16().collect();
    let trimmed = units.len().min(31);
    for (index, unit) in units[..trimmed].iter().enumerate() {
        dir[at + index * 2..at + index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    dir[at + 0x40..at + 0x42].copy_from_slice(&(((trimmed + 1) * 2) as u16).to_le_bytes());
    dir[at + 0x42] = kind;
    dir[at + 0x43] = 1; // black
    dir[at + 0x44..at + 0x48].copy_from_slice(&NOSTREAM.to_le_bytes()); // left
    dir[at + 0x48..at + 0x4C].copy_from_slice(&NOSTREAM.to_le_bytes()); // right（兄弟链稍后覆盖）
    dir[at + 0x4C..at + 0x50].copy_from_slice(&child.to_le_bytes());
    dir[at + 0x70..at + 0x74].copy_from_slice(&start.to_le_bytes());
    dir[at + 0x74..at + 0x78].copy_from_slice(&(size as u32).to_le_bytes());
    dir[at + 0x78..at + 0x7C].copy_from_slice(&((size >> 32) as u32).to_le_bytes());
}

fn write_header(file: &mut [u8], fat_sectors: u32, dir_start: u32, minifat_start: u32, minifat_sectors: u32, fat_start: u32) {
    file[0..8].copy_from_slice(&SIGNATURE);
    file[0x14..0x16].copy_from_slice(&0x003Eu16.to_le_bytes());
    file[0x16..0x18].copy_from_slice(&0x0003u16.to_le_bytes());
    file[0x18..0x1A].copy_from_slice(&0xFFFEu16.to_le_bytes());
    file[0x1A..0x1C].copy_from_slice(&(SECTOR.trailing_zeros() as u16).to_le_bytes());
    file[0x1C..0x1E].copy_from_slice(&(MINI.trailing_zeros() as u16).to_le_bytes());
    file[0x28..0x2C].copy_from_slice(&fat_sectors.to_le_bytes());
    file[0x2C..0x30].copy_from_slice(&dir_start.to_le_bytes());
    file[0x34..0x38].copy_from_slice(&(MAX_MINI as u32).to_le_bytes());
    file[0x38..0x3C].copy_from_slice(&minifat_start.to_le_bytes());
    file[0x3C..0x40].copy_from_slice(&minifat_sectors.to_le_bytes());
    file[0x40..0x44].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
    file[0x44..0x48].copy_from_slice(&0u32.to_le_bytes());
    for index in 0..DIFAT_INLINE {
        let value = if (index as u32) < fat_sectors { fat_start + index as u32 } else { FREESECT };
        file[0x48 + index * 4..0x4C + index * 4].copy_from_slice(&value.to_le_bytes());
    }
}

/// 测试用回读器：解析目录并返回 `path → bytes`（存储内流路径为 `存储/流`）。
#[cfg(test)]
pub(crate) fn read_streams(bytes: &[u8]) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    use std::collections::BTreeMap;

    if bytes.len() < SECTOR || bytes[0..8] != SIGNATURE {
        return Err(InstallerError::Verify("bad CFB signature".into()));
    }
    let header_u32 = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let fat_sectors = header_u32(0x28) as usize;
    let dir_start = header_u32(0x2C);
    let minifat_start = header_u32(0x38);
    let sector = |id: u32| &bytes[SECTOR * (1 + id as usize)..SECTOR * (2 + id as usize)];
    let chain = |table: &[u32], mut id: u32| -> Vec<u32> {
        let mut out = Vec::new();
        while id != ENDOFCHAIN && id != FREESECT {
            out.push(id);
            id = table[id as usize];
        }
        out
    };

    let mut fat: Vec<u32> = Vec::with_capacity(fat_sectors * (SECTOR / 4));
    for index in 0..fat_sectors {
        let id = header_u32(0x48 + index * 4);
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
    // root 目录项在 dir_start 扇区偏移 0；root.start 在项内 0x70，即 mini 流数据链首。
    let root_start = u32::from_le_bytes(sector(dir_start)[0x70..0x74].try_into().unwrap());
    let root_sectors = chain(&fat, root_start);
    let mini_data = |start: u32, size: usize| -> Vec<u8> {
        let mut out = Vec::with_capacity(size);
        for id in chain(&minifat, start) {
            let sector_index = id as usize / (SECTOR / MINI);
            let offset = (id as usize % (SECTOR / MINI)) * MINI;
            let at = SECTOR * (1 + root_sectors[sector_index] as usize) + offset;
            out.extend_from_slice(&bytes[at..at + MINI]);
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
            let start = u32::from_le_bytes(item[0x70..0x74].try_into().unwrap());
            let size = u32::from_le_bytes(item[0x74..0x78].try_into().unwrap()) as u64;
            entries.push((String::from_utf16_lossy(&units), kind, left, right, child, start, size));
        }
    }
    fn walk(entries: &[(String, u8, u32, u32, u32, u32, u64)], id: u32, out: &mut Vec<u32>) {
        if id == NOSTREAM {
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
        else if (size as usize) < MAX_MINI {
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
}
