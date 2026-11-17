//! MS-CAB writer：单 folder、MSZIP 压缩档（type=1），但每个 32768 字节块
//! 用 deflate *stored* 块原样搬运（不真压缩）。
//!
//! MSI 只认 cabinet 这一条提取路径（`Media.Cabinet="#FileData"`），stored
//! MSZIP 把压缩实现风险压到零：解压偏移 = 原始偏移，块头 `CK` + 1 字节
//! `BFINAL/BTYPE=00` + `LEN/NLEN` + 原样字节。体积 ≈ payload + 0.03%。
//! 真压缩（fixed-Huffman deflate）留后续独立块，不阻塞本轮。

use crate::crc32::crc32;

/// MSZIP 块解压尺寸（除末块外每块必须恰好为该值）。
const MSZIP_BLOCK: usize = 32768;

/// 一个 cab 内文件。
pub(crate) struct CabEntry<'a> {
    /// cab 内路径（`\` 分隔；MSI 场景为平铺文件名）。
    pub name: &'a str,
    /// 原始字节。
    pub data: &'a [u8],
}

/// 构建完整 cabinet 字节流。
pub(crate) fn build_cab(entries: &[CabEntry<'_>], set_id: u32) -> Vec<u8> {
    // folder 数据 = 各文件按序拼接；stored MSZIP 下解压偏移 = 原始偏移。
    let folder: Vec<u8> = entries.iter().flat_map(|entry| entry.data.iter().copied()).collect();
    let block_count = if folder.is_empty() { 0 } else { folder.len().div_ceil(MSZIP_BLOCK) };

    // 文件表（16B 定长 + NUL 结尾名）。
    let mut file_table = Vec::new();
    let mut offset = 0u32;
    for entry in entries {
        let name = format!("{}\0", entry.name);
        file_table.extend_from_slice(&(entry.data.len() as u32).to_le_bytes()); // cbFile
        file_table.extend_from_slice(&offset.to_le_bytes()); // uoffFolderStart
        file_table.extend_from_slice(&0x0021u16.to_le_bytes()); // ftDate: 1980-01-01
        file_table.extend_from_slice(&0u16.to_le_bytes()); // ftTime
        file_table.extend_from_slice(&0u32.to_le_bytes()); // iFolder
        file_table.extend_from_slice(&0u32.to_le_bytes()); // atAttrib
        file_table.extend_from_slice(name.as_bytes());
        offset += entry.data.len() as u32;
    }

    // 数据区：每块 = "CK" + deflate stored 块（1 字节头 + LEN + NLEN + 原样）。
    let mut data_area = Vec::with_capacity(folder.len() + block_count * 7);
    for index in 0..block_count {
        let chunk = &folder[index * MSZIP_BLOCK..((index + 1) * MSZIP_BLOCK).min(folder.len())];
        let final_block = index + 1 == block_count;
        data_area.extend_from_slice(b"CK");
        data_area.push(if final_block { 0x01 } else { 0x00 }); // BFINAL, BTYPE=00
        data_area.extend_from_slice(&(chunk.len() as u16).to_le_bytes()); // LEN
        data_area.extend_from_slice(&(!chunk.len() as u16).to_le_bytes()); // NLEN
        data_area.extend_from_slice(chunk);
    }

    let header_size = 44usize;
    let folder_size = 16usize;
    let data_start = header_size + folder_size + file_table.len();
    let total = data_start + data_area.len();

    let mut cab = Vec::with_capacity(total);
    cab.extend_from_slice(b"MSCF");
    cab.extend_from_slice(&0u32.to_le_bytes()); // reserved1
    cab.extend_from_slice(&(total as u32).to_le_bytes()); // cbCabinet
    cab.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    cab.extend_from_slice(&(entries.len() as u32).to_le_bytes()); // cfFiles
    cab.extend_from_slice(&1u32.to_le_bytes()); // cfFolders
    cab.extend_from_slice(&0u32.to_le_bytes()); // flags
    cab.extend_from_slice(&set_id.to_le_bytes()); // setID
    cab.extend_from_slice(&0u32.to_le_bytes()); // iCabinet
    cab.extend_from_slice(&(folder_size as u32).to_le_bytes()); // cbCFHeader
    cab.extend_from_slice(&0u16.to_le_bytes()); // cbCFFolder
    cab.extend_from_slice(&0u16.to_le_bytes()); // cbCFData
    // CFFOLDER。
    cab.extend_from_slice(&(data_start as u32).to_le_bytes()); // coffCabStart
    cab.extend_from_slice(&(block_count as u32).to_le_bytes()); // cCFData
    cab.extend_from_slice(&1u16.to_le_bytes()); // typeCompress = MSZIP
    cab.extend_from_slice(&0u16.to_le_bytes()); // reserved
    cab.extend_from_slice(&crc32(&folder).to_le_bytes()); // folder CRC（未压缩数据）
    cab.extend_from_slice(&file_table);
    cab.extend_from_slice(&data_area);
    cab
}

/// 测试用回读：解析 CFFILE 表并把 stored MSZIP 数据区还原成 folder 字节。
#[cfg(test)]
pub(crate) fn read_cab(cab: &[u8]) -> (Vec<(String, u32, u32)>, Vec<u8>) {
    let field_u32 = |at: usize| u32::from_le_bytes(cab[at..at + 4].try_into().unwrap());
    let cf_files = field_u32(16) as usize;
    let data_start = field_u32(44) as usize; // CFFOLDER.coffCabStart
    let mut cursor = 44 + 16;
    let mut files = Vec::with_capacity(cf_files);
    for _ in 0..cf_files {
        let cb_file = field_u32(cursor);
        let folder_start = field_u32(cursor + 4);
        cursor += 20; // cbFile + uoffFolderStart + ftDate + ftTime + iFolder + atAttrib
        let name_end = cab[cursor..].iter().position(|byte| *byte == 0).expect("NUL-terminated name");
        let name = String::from_utf8_lossy(&cab[cursor..cursor + name_end]).into_owned();
        cursor += name_end + 1;
        files.push((name, cb_file, folder_start));
    }
    let mut folder = Vec::new();
    let mut at = data_start;
    while at < cab.len() {
        assert_eq!(&cab[at..at + 2], b"CK", "CFDATA signature");
        at += 2;
        let header = cab[at];
        assert_eq!(header & 0x06, 0, "stored block type");
        at += 1;
        let len = u16::from_le_bytes(cab[at..at + 2].try_into().unwrap()) as usize;
        let nlen = u16::from_le_bytes(cab[at + 2..at + 4].try_into().unwrap()) as usize;
        assert_eq!(len ^ 0xFFFF, nlen, "NLEN");
        at += 4;
        folder.extend_from_slice(&cab[at..at + len]);
        at += len;
    }
    (files, folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_entries_and_offsets() {
        let first = vec![7u8; 1000];
        let second: Vec<u8> = (0..40000u32).map(|value| value as u8).collect();
        let entries = [CabEntry { name: "app.exe", data: &first }, CabEntry { name: "index.html", data: &second }];
        let cab = build_cab(&entries, 0x1234_5678);
        assert_eq!(&cab[0..4], b"MSCF");
        let (files, folder) = read_cab(&cab);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "app.exe");
        assert_eq!(files[0].1, 1000);
        assert_eq!(files[0].2, 0);
        assert_eq!(files[1].0, "index.html");
        assert_eq!(files[1].2, 1000);
        let mut expected = Vec::new();
        expected.extend_from_slice(&first);
        expected.extend_from_slice(&second);
        assert_eq!(folder, expected);
    }

    #[test]
    fn empty_payload_yields_zero_blocks() {
        let cab = build_cab(&[], 1);
        assert_eq!(&cab[0..4], b"MSCF");
        let (files, folder) = read_cab(&cab);
        assert!(files.is_empty());
        assert!(folder.is_empty());
    }

    #[test]
    fn block_boundaries_exact() {
        let data: Vec<u8> = (0..(MSZIP_BLOCK * 2 + 5)).map(|value| (value % 251) as u8).collect();
        let entries = [CabEntry { name: "big.bin", data: &data }];
        let cab = build_cab(&entries, 9);
        let (_files, folder) = read_cab(&cab);
        assert_eq!(folder, data);
        // 3 块：两块满 32768，末块 5 字节。
        assert_eq!(u32::from_le_bytes(cab[48..52].try_into().unwrap()), 3);
    }
}
