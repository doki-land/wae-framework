//! MD5（RFC 1321）：deb `md5sums`、rpm `FILEDIGESTS` 与 MSI GUID 确定性派生共用；
//! 不引第三方 crate。

/// 计算 MD5，返回 16 字节。
pub(crate) fn md5(data: &[u8]) -> [u8; 16] {
    let mut state = Md5::new();
    state.update(data);
    state.finalize()
}

/// 流式 MD5 累加器。
pub(crate) struct Md5 {
    state: [u32; 4],
    buffer: [u8; 64],
    buffered: usize,
    len_bits: u64,
}

const S: [u32; 4] = [7, 12, 17, 22];
const S2: [u32; 4] = [5, 9, 14, 20];
const S3: [u32; 4] = [4, 11, 16, 23];
const S4: [u32; 4] = [6, 10, 15, 21];

const K: [u32; 64] = [
    0xD76AA478, 0xE8C7B756, 0x242070DB, 0xC1BDCEEE, 0xF57C0FAF, 0x4787C62A, 0xA8304613, 0xFD469501, 0x698098D8, 0x8B44F7AF, 0xFFFF5BB1,
    0x895CD7BE, 0x6B901122, 0xFD987193, 0xA679438E, 0x49B40821, 0xF61E2562, 0xC040B340, 0x265E5A51, 0xE9B6C7AA, 0xD62F105D, 0x02441453,
    0xD8A1E681, 0xE7D3FBC8, 0x21E1CDE6, 0xC33707D6, 0xF4D50D87, 0x455A14ED, 0xA9E3E905, 0xFCEFA3F8, 0x676F02D9, 0x8D2A4C8A, 0xFFFA3942,
    0x8771F681, 0x6D9D6122, 0xFDE5380C, 0xA4BEEA44, 0x4BDECFA9, 0xF6BB4B60, 0xBEBFBC70, 0x289B7EC6, 0xEAA127FA, 0xD4EF3085, 0x04881D05,
    0xD9D4D039, 0xE6DB99E5, 0x1FA27CF8, 0xC4AC5665, 0xF4292244, 0x432AFF97, 0xAB9423A7, 0xFC93A039, 0x655B59C3, 0x8F0CCC92, 0xFFEFF47D,
    0x85845DD1, 0x6FA87E4F, 0xFE2CE6E0, 0xA3014314, 0x4E0811A1, 0xF7537E82, 0xBD3AF235, 0x2AD7D2BB, 0xEB86D391,
];

impl Md5 {
    fn new() -> Self {
        Self { state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476], buffer: [0u8; 64], buffered: 0, len_bits: 0 }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.len_bits += (data.len() as u64) * 8;
        while !data.is_empty() {
            let take = (64 - self.buffered).min(data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered == 64 {
                self.compress();
                self.buffered = 0;
            }
        }
    }

    fn finalize(mut self) -> [u8; 16] {
        let total_bits = self.len_bits;
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0x00]);
        }
        self.update(&total_bits.to_le_bytes());
        let mut out = [0u8; 16];
        for (chunk, value) in out.chunks_mut(4).zip(self.state) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn compress(&mut self) {
        let mut words = [0u32; 16];
        for (index, word) in words.iter_mut().enumerate() {
            *word = u32::from_le_bytes(self.buffer[index * 4..index * 4 + 4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (self.state[0], self.state[1], self.state[2], self.state[3]);
        for (i, k) in K.iter().enumerate() {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let shift = match i / 16 {
                0 => S[i % 4],
                1 => S2[i % 4],
                2 => S3[i % 4],
                _ => S4[i % 4],
            };
            let sum = a.wrapping_add(f).wrapping_add(*k).wrapping_add(words[g]).rotate_left(shift).wrapping_add(b);
            a = d;
            d = c;
            c = b;
            b = sum;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
    }
}

/// MD5 前 16 字节格式化为大写 GUID（版本/变体位按 v4 风格固定，保证确定性即可）。
pub(crate) fn md5_guid(seed: &str) -> String {
    let digest = md5(seed.as_bytes());
    let mut bytes = digest;
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!("{{{0}-{1}-{2}-{3}-{4}}}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // 期望值以 Node crypto 与 RFC 附录钉死。
        assert_eq!(md5(b""), [0xD4, 0x1D, 0x8C, 0xD9, 0x8F, 0x00, 0xB2, 0x04, 0xE9, 0x80, 0x09, 0x98, 0xEC, 0xF8, 0x42, 0x7E]);
        assert_eq!(md5(b"abc"), [0x90, 0x01, 0x50, 0x98, 0x3C, 0xD2, 0x4F, 0xB0, 0xD6, 0x96, 0x3F, 0x7D, 0x28, 0xE1, 0x7F, 0x72]);
        let quick: &[u8] = b"The quick brown fox jumps over the lazy dog";
        assert_eq!(md5(quick), [0x9E, 0x10, 0x7D, 0x9D, 0x37, 0x2B, 0xB6, 0x82, 0x6B, 0xD8, 0x1D, 0x35, 0x42, 0xA4, 0x19, 0xD6]);
    }

    #[test]
    fn single_byte_vector() {
        // MD5("a") = 0cc175b9c0f1b6a831c399e269772661
        assert_eq!(md5(b"a"), [0x0C, 0xC1, 0x75, 0xB9, 0xC0, 0xF1, 0xB6, 0xA8, 0x31, 0xC3, 0x99, 0xE2, 0x69, 0x77, 0x26, 0x61]);
    }

    #[test]
    fn guid_shape_and_determinism() {
        let a = md5_guid("component/MyJarvis");
        let b = md5_guid("component/MyJarvis");
        assert_eq!(a, b);
        assert_eq!(a.len(), 38);
        assert!(a.starts_with('{') && a.ends_with('}'));
        assert_ne!(md5_guid("other"), a);
    }
}
