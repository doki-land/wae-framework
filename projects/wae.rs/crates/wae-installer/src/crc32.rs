//! 表驱动 CRC-32（IEEE 802.3 多项式 `0xEDB88320`），cab、gzip 与 deb
//! 校验共用；不引第三方 crate。

/// 单遍计算 CRC-32。
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut state = Crc32::new();
    state.update(data);
    state.finish()
}

/// 流式 CRC-32 累加器。
#[derive(Debug, Default)]
pub(crate) struct Crc32 {
    crc: u32,
}

impl Crc32 {
    /// 新建累加器。
    pub(crate) fn new() -> Self {
        Self { crc: 0xFFFF_FFFF }
    }

    /// 追加数据。
    pub(crate) fn update(&mut self, data: &[u8]) {
        for byte in data {
            let index = ((self.crc ^ u32::from(*byte)) & 0xFF) as usize;
            self.crc = (self.crc >> 8) ^ TABLE[index];
        }
    }

    /// 完成并返回校验值。
    pub(crate) fn finish(&self) -> u32 {
        self.crc ^ 0xFFFF_FFFF
    }
}

/// 编译期生成的查表。
static TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // 标准 CRC-32 测试向量。
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn streaming_matches_one_shot() {
        let data: Vec<u8> = (0..1000u32).map(|value| value as u8).collect();
        let mut state = Crc32::new();
        for chunk in data.chunks(97) {
            state.update(chunk);
        }
        assert_eq!(state.finish(), crc32(&data));
    }
}
