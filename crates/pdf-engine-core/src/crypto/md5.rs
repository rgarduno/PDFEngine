//! Pure Rust implementation of the MD5 message-digest algorithm (RFC 1321).
//! Used in ISO 32000-1 §7.6 PDF Standard Security Handler (Revisions 2, 3, and 4)
//! for encryption key derivation and password verification.

/// Computes the 128-bit (16-byte) MD5 digest of `data`.
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut state = [0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32];

    let bit_len = (data.len() as u64).wrapping_mul(8);

    // Padding: append 0x80, then zeros until len % 64 == 56, then 64-bit little-endian bit length.
    let mut padded = Vec::with_capacity(data.len() + 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0x00);
    }
    padded.extend_from_slice(&bit_len.to_le_bytes());

    // Process each 64-byte block
    for chunk in padded.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            let offset = i * 4;
            *word = u32::from_le_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];

        // Round 1
        macro_rules! ff {
            ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
                $a = $b.wrapping_add(
                    ($a.wrapping_add(($b & $c) | ((!$b) & $d))
                        .wrapping_add(m[$k])
                        .wrapping_add($i))
                    .rotate_left($s),
                );
            };
        }

        ff!(a, b, c, d, 0, 7, 0xd76aa478);
        ff!(d, a, b, c, 1, 12, 0xe8c7b756);
        ff!(c, d, a, b, 2, 17, 0x242070db);
        ff!(b, c, d, a, 3, 22, 0xc1bdceee);
        ff!(a, b, c, d, 4, 7, 0xf57c0faf);
        ff!(d, a, b, c, 5, 12, 0x4787c62a);
        ff!(c, d, a, b, 6, 17, 0xa8304613);
        ff!(b, c, d, a, 7, 22, 0xfd469501);
        ff!(a, b, c, d, 8, 7, 0x698098d8);
        ff!(d, a, b, c, 9, 12, 0x8b44f7af);
        ff!(c, d, a, b, 10, 17, 0xffff5bb1);
        ff!(b, c, d, a, 11, 22, 0x895cd7be);
        ff!(a, b, c, d, 12, 7, 0x6b901122);
        ff!(d, a, b, c, 13, 12, 0xfd987193);
        ff!(c, d, a, b, 14, 17, 0xa679438e);
        ff!(b, c, d, a, 15, 22, 0x49b40821);

        // Round 2
        macro_rules! gg {
            ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
                $a = $b.wrapping_add(
                    ($a.wrapping_add(($b & $d) | ($c & (!$d)))
                        .wrapping_add(m[$k])
                        .wrapping_add($i))
                    .rotate_left($s),
                );
            };
        }

        gg!(a, b, c, d, 1, 5, 0xf61e2562);
        gg!(d, a, b, c, 6, 9, 0xc040b340);
        gg!(c, d, a, b, 11, 14, 0x265e5a51);
        gg!(b, c, d, a, 0, 20, 0xe9b6c7aa);
        gg!(a, b, c, d, 5, 5, 0xd62f105d);
        gg!(d, a, b, c, 10, 9, 0x02441453);
        gg!(c, d, a, b, 15, 14, 0xd8a1e681);
        gg!(b, c, d, a, 4, 20, 0xe7d3fbc8);
        gg!(a, b, c, d, 9, 5, 0x21e1cde6);
        gg!(d, a, b, c, 14, 9, 0xc33707d6);
        gg!(c, d, a, b, 3, 14, 0xf4d50d87);
        gg!(b, c, d, a, 8, 20, 0x455a14ed);
        gg!(a, b, c, d, 13, 5, 0xa9e3e905);
        gg!(d, a, b, c, 2, 9, 0xfcefa3f8);
        gg!(c, d, a, b, 7, 14, 0x676f02d9);
        gg!(b, c, d, a, 12, 20, 0x8d2a4c8a);

        // Round 3
        macro_rules! hh {
            ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
                $a = $b.wrapping_add(
                    ($a.wrapping_add($b ^ $c ^ $d)
                        .wrapping_add(m[$k])
                        .wrapping_add($i))
                    .rotate_left($s),
                );
            };
        }

        hh!(a, b, c, d, 5, 4, 0xfffa3942);
        hh!(d, a, b, c, 8, 11, 0x8771f681);
        hh!(c, d, a, b, 11, 16, 0x6d9d6122);
        hh!(b, c, d, a, 14, 23, 0xfde5380c);
        hh!(a, b, c, d, 1, 4, 0xa4beea44);
        hh!(d, a, b, c, 4, 11, 0x4bdecfa9);
        hh!(c, d, a, b, 7, 16, 0xf6bb4b60);
        hh!(b, c, d, a, 10, 23, 0xbebfbc70);
        hh!(a, b, c, d, 13, 4, 0x289b7ec6);
        hh!(d, a, b, c, 0, 11, 0xeaa127fa);
        hh!(c, d, a, b, 3, 16, 0xd4ef3085);
        hh!(b, c, d, a, 6, 23, 0x04881d05);
        hh!(a, b, c, d, 9, 4, 0xd9d4d039);
        hh!(d, a, b, c, 12, 11, 0xe6db99e5);
        hh!(c, d, a, b, 15, 16, 0x1fa27cf8);
        hh!(b, c, d, a, 2, 23, 0xc4ac5665);

        // Round 4
        macro_rules! ii {
            ($a:expr, $b:expr, $c:expr, $d:expr, $k:expr, $s:expr, $i:expr) => {
                $a = $b.wrapping_add(
                    ($a.wrapping_add($c ^ ($b | (!$d)))
                        .wrapping_add(m[$k])
                        .wrapping_add($i))
                    .rotate_left($s),
                );
            };
        }

        ii!(a, b, c, d, 0, 6, 0xf4292244);
        ii!(d, a, b, c, 7, 10, 0x432aff97);
        ii!(c, d, a, b, 14, 15, 0xab9423a7);
        ii!(b, c, d, a, 5, 21, 0xfc93a039);
        ii!(a, b, c, d, 12, 6, 0x655b59c3);
        ii!(d, a, b, c, 3, 10, 0x8f0ccc92);
        ii!(c, d, a, b, 10, 15, 0xffeff47d);
        ii!(b, c, d, a, 1, 21, 0x85845dd1);
        ii!(a, b, c, d, 8, 6, 0x6fa87e4f);
        ii!(d, a, b, c, 15, 10, 0xfe2ce6e0);
        ii!(c, d, a, b, 6, 15, 0xa3014314);
        ii!(b, c, d, a, 13, 21, 0x4e0811a1);
        ii!(a, b, c, d, 4, 6, 0xf7537e82);
        ii!(d, a, b, c, 11, 10, 0xbd3af235);
        ii!(c, d, a, b, 2, 15, 0x2ad7d2bb);
        ii!(b, c, d, a, 9, 21, 0xeb86d391);

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }

    let mut digest = [0u8; 16];
    digest[0..4].copy_from_slice(&state[0].to_le_bytes());
    digest[4..8].copy_from_slice(&state[1].to_le_bytes());
    digest[8..12].copy_from_slice(&state[2].to_le_bytes());
    digest[12..16].copy_from_slice(&state[3].to_le_bytes());
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md5_rfc1321_test_vectors() {
        assert_eq!(hex::encode(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex::encode(md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex::encode(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex::encode(md5(b"message digest")),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
        assert_eq!(
            hex::encode(md5(b"abcdefghijklmnopqrstuvwxyz")),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }

    mod hex {
        pub fn encode(bytes: [u8; 16]) -> String {
            bytes.iter().map(|b| format!("{:02x}", b)).collect()
        }
    }
}
