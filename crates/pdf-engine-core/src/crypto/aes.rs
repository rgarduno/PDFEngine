//! Pure Rust implementation of the Advanced Encryption Standard (AES / Rijndael, FIPS PUB 197).
//! Supports 128-bit and 256-bit keys in Cipher Block Chaining (CBC) mode with PKCS#7 padding,
//! conforming to ISO 32000-1 §7.6.2 and ISO 32000-2 §7.6.

use crate::error::{PdfError, PdfResult};

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const RCON: [u32; 11] = [
    0x00000000, 0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000,
    0x20000000, 0x40000000, 0x80000000, 0x1b000000, 0x36000000,
];

#[inline(always)]
fn sub_word(w: u32) -> u32 {
    let b = w.to_be_bytes();
    u32::from_be_bytes([
        SBOX[b[0] as usize],
        SBOX[b[1] as usize],
        SBOX[b[2] as usize],
        SBOX[b[3] as usize],
    ])
}

#[inline(always)]
fn rot_word(w: u32) -> u32 {
    w.rotate_left(8)
}

#[inline(always)]
fn xtime(x: u8) -> u8 {
    (x << 1) ^ (if (x & 0x80) != 0 { 0x1b } else { 0 })
}

#[inline(always)]
fn mul(a: u8, mut b: u8) -> u8 {
    let mut p = 0u8;
    let mut aa = a;
    while b != 0 {
        if (b & 1) != 0 {
            p ^= aa;
        }
        aa = xtime(aa);
        b >>= 1;
    }
    p
}

/// Key expansion and round keys for AES.
#[derive(Clone)]
pub struct AesKey {
    round_keys: Vec<u32>,
    rounds: usize,
}

impl AesKey {
    /// Creates round keys from 128-bit (16-byte) or 256-bit (32-byte) key.
    pub fn new(key: &[u8]) -> PdfResult<Self> {
        let (nk, nr) = match key.len() {
            16 => (4, 10),
            32 => (8, 14),
            other => {
                return Err(PdfError::CryptographyError(format!(
                    "Unsupported AES key length: {} bytes (must be 16 or 32)",
                    other
                )))
            }
        };

        let total_words = 4 * (nr + 1);
        let mut w = vec![0u32; total_words];

        for i in 0..nk {
            let offset = i * 4;
            w[i] = u32::from_be_bytes([
                key[offset],
                key[offset + 1],
                key[offset + 2],
                key[offset + 3],
            ]);
        }

        for i in nk..total_words {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = sub_word(rot_word(temp)) ^ RCON[i / nk];
            } else if nk > 6 && i % nk == 4 {
                temp = sub_word(temp);
            }
            w[i] = w[i - nk] ^ temp;
        }

        Ok(Self {
            round_keys: w,
            rounds: nr,
        })
    }

    /// Encrypts a single 16-byte block in place.
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        let mut state = [[0u8; 4]; 4];
        for r in 0..4 {
            for c in 0..4 {
                state[r][c] = block[r + 4 * c];
            }
        }

        // Initial Round: AddRoundKey
        self.add_round_key(&mut state, 0);

        // Main Rounds
        for round in 1..self.rounds {
            // SubBytes
            for r in 0..4 {
                for c in 0..4 {
                    state[r][c] = SBOX[state[r][c] as usize];
                }
            }

            // ShiftRows
            state[1].rotate_left(1);
            state[2].rotate_left(2);
            state[3].rotate_left(3);

            // MixColumns
            for c in 0..4 {
                let s0 = state[0][c];
                let s1 = state[1][c];
                let s2 = state[2][c];
                let s3 = state[3][c];

                state[0][c] = xtime(s0) ^ (s1 ^ xtime(s1)) ^ s2 ^ s3;
                state[1][c] = s0 ^ xtime(s1) ^ (s2 ^ xtime(s2)) ^ s3;
                state[2][c] = s0 ^ s1 ^ xtime(s2) ^ (s3 ^ xtime(s3));
                state[3][c] = (s0 ^ xtime(s0)) ^ s1 ^ s2 ^ xtime(s3);
            }

            self.add_round_key(&mut state, round);
        }

        // Final Round (no MixColumns)
        for r in 0..4 {
            for c in 0..4 {
                state[r][c] = SBOX[state[r][c] as usize];
            }
        }

        state[1].rotate_left(1);
        state[2].rotate_left(2);
        state[3].rotate_left(3);

        self.add_round_key(&mut state, self.rounds);

        for r in 0..4 {
            for c in 0..4 {
                block[r + 4 * c] = state[r][c];
            }
        }
    }

    /// Decrypts a single 16-byte block in place.
    pub fn decrypt_block(&self, block: &mut [u8; 16]) {
        let mut state = [[0u8; 4]; 4];
        for r in 0..4 {
            for c in 0..4 {
                state[r][c] = block[r + 4 * c];
            }
        }

        self.add_round_key(&mut state, self.rounds);

        for round in (1..self.rounds).rev() {
            // InvShiftRows
            state[1].rotate_right(1);
            state[2].rotate_right(2);
            state[3].rotate_right(3);

            // InvSubBytes
            for r in 0..4 {
                for c in 0..4 {
                    state[r][c] = INV_SBOX[state[r][c] as usize];
                }
            }

            self.add_round_key(&mut state, round);

            // InvMixColumns
            for c in 0..4 {
                let s0 = state[0][c];
                let s1 = state[1][c];
                let s2 = state[2][c];
                let s3 = state[3][c];

                state[0][c] = mul(s0, 0x0e) ^ mul(s1, 0x0b) ^ mul(s2, 0x0d) ^ mul(s3, 0x09);
                state[1][c] = mul(s0, 0x09) ^ mul(s1, 0x0e) ^ mul(s2, 0x0b) ^ mul(s3, 0x0d);
                state[2][c] = mul(s0, 0x0d) ^ mul(s1, 0x09) ^ mul(s2, 0x0e) ^ mul(s3, 0x0b);
                state[3][c] = mul(s0, 0x0b) ^ mul(s1, 0x0d) ^ mul(s2, 0x09) ^ mul(s3, 0x0e);
            }
        }

        // Final round
        state[1].rotate_right(1);
        state[2].rotate_right(2);
        state[3].rotate_right(3);

        for r in 0..4 {
            for c in 0..4 {
                state[r][c] = INV_SBOX[state[r][c] as usize];
            }
        }

        self.add_round_key(&mut state, 0);

        for r in 0..4 {
            for c in 0..4 {
                block[r + 4 * c] = state[r][c];
            }
        }
    }

    #[inline(always)]
    fn add_round_key(&self, state: &mut [[u8; 4]; 4], round: usize) {
        let base = round * 4;
        for c in 0..4 {
            let key_word = self.round_keys[base + c].to_be_bytes();
            for r in 0..4 {
                state[r][c] ^= key_word[r];
            }
        }
    }
}

/// Encrypts plaintext with AES in Cipher Block Chaining (CBC) mode with PKCS#7 padding.
/// Conforming to ISO 32000-1 §7.6.2, the 16-byte random IV is prepended to the ciphertext.
pub fn aes_cbc_encrypt(key: &[u8], iv: &[u8; 16], plaintext: &[u8]) -> PdfResult<Vec<u8>> {
    let cipher = AesKey::new(key)?;

    // PKCS#7 padding
    let pad_len = 16 - (plaintext.len() % 16);
    let mut buffer = Vec::with_capacity(16 + plaintext.len() + pad_len);
    
    // Output starts with IV
    buffer.extend_from_slice(iv);

    let mut current_block = *iv;

    // Process full blocks and padded block
    let total_len = plaintext.len() + pad_len;
    for offset in (0..total_len).step_by(16) {
        let mut block = [pad_len as u8; 16];
        let chunk_len = (plaintext.len().saturating_sub(offset)).min(16);
        if chunk_len > 0 {
            block[..chunk_len].copy_from_slice(&plaintext[offset..offset + chunk_len]);
        }

        // CBC XOR with previous ciphertext (or IV)
        for i in 0..16 {
            block[i] ^= current_block[i];
        }

        cipher.encrypt_block(&mut block);
        current_block = block;
        buffer.extend_from_slice(&block);
    }

    Ok(buffer)
}

/// Decrypts ciphertext with AES in CBC mode with PKCS#7 padding removal.
/// The input data must contain the 16-byte IV as its first 16 bytes.
pub fn aes_cbc_decrypt(key: &[u8], ciphertext_with_iv: &[u8]) -> PdfResult<Vec<u8>> {
    if ciphertext_with_iv.len() < 32 || (ciphertext_with_iv.len() % 16) != 0 {
        return Err(PdfError::CryptographyError(format!(
            "Invalid AES CBC ciphertext length: {} bytes (must be >= 32 and multiple of 16)",
            ciphertext_with_iv.len()
        )));
    }

    let cipher = AesKey::new(key)?;
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&ciphertext_with_iv[..16]);

    let blocks = &ciphertext_with_iv[16..];
    let mut plaintext = Vec::with_capacity(blocks.len());

    let mut prev_block = iv;
    for chunk in blocks.chunks_exact(16) {
        let mut block = [0u8; 16];
        block.copy_from_slice(chunk);

        let mut decrypted = block;
        cipher.decrypt_block(&mut decrypted);

        for i in 0..16 {
            decrypted[i] ^= prev_block[i];
        }
        prev_block = block;

        plaintext.extend_from_slice(&decrypted);
    }

    // Remove and validate PKCS#7 padding
    if let Some(&pad_val) = plaintext.last() {
        let pad_len = pad_val as usize;
        if pad_len == 0 || pad_len > 16 || pad_len > plaintext.len() {
            return Err(PdfError::CryptographyError("Invalid PKCS#7 padding".to_string()));
        }

        for &b in &plaintext[plaintext.len() - pad_len..] {
            if b != pad_val {
                return Err(PdfError::CryptographyError("Malformed PKCS#7 padding bytes".to_string()));
            }
        }

        plaintext.truncate(plaintext.len() - pad_len);
        Ok(plaintext)
    } else {
        Err(PdfError::CryptographyError("Empty decrypted plaintext".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes_128_nist_ecb_roundtrip() {
        // NIST SP 800-38A test vector for AES-128
        let key = hex::decode("2b7e151628aed2a6abf7158809cf4f3c");
        let plaintext = hex::decode("6bc1bee22e409f96e93d7e117393172a");

        let cipher = AesKey::new(&key).unwrap();
        let mut block = [0u8; 16];
        block.copy_from_slice(&plaintext);

        cipher.encrypt_block(&mut block);
        assert_eq!(
            hex::encode(&block),
            "3ad77bb40d7a3660a89ecaf32466ef97"
        );

        cipher.decrypt_block(&mut block);
        assert_eq!(hex::encode(&block), "6bc1bee22e409f96e93d7e117393172a");
    }

    #[test]
    fn test_aes_256_nist_ecb_roundtrip() {
        // NIST SP 800-38A test vector for AES-256
        let key = hex::decode("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
        let plaintext = hex::decode("6bc1bee22e409f96e93d7e117393172a");

        let cipher = AesKey::new(&key).unwrap();
        let mut block = [0u8; 16];
        block.copy_from_slice(&plaintext);

        cipher.encrypt_block(&mut block);
        assert_eq!(
            hex::encode(&block),
            "f3eed1bdb5d2a03c064b5a7e3db181f8"
        );

        cipher.decrypt_block(&mut block);
        assert_eq!(hex::encode(&block), "6bc1bee22e409f96e93d7e117393172a");
    }

    #[test]
    fn test_aes_cbc_encrypt_decrypt_roundtrip() {
        let key = b"0123456789abcdef"; // 16 bytes = 128 bit
        let iv = [0x42u8; 16];
        let secret_data = b"Enterprise PDF Engine: Secret Contract Value = $1,500,000.00 USD";

        let encrypted = aes_cbc_encrypt(key, &iv, secret_data).unwrap();
        assert_ne!(&encrypted[16..], secret_data);
        assert_eq!(&encrypted[..16], &iv);

        let decrypted = aes_cbc_decrypt(key, &encrypted).unwrap();
        assert_eq!(decrypted, secret_data);
    }

    #[test]
    fn test_aes_256_cbc_encrypt_decrypt_roundtrip() {
        let key = b"0123456789abcdef0123456789abcdef"; // 32 bytes = 256 bit
        let iv = [0x99u8; 16];
        let secret_data = b"Confidential Medical Record: Patient exhibits full remission.";

        let encrypted = aes_cbc_encrypt(key, &iv, secret_data).unwrap();
        let decrypted = aes_cbc_decrypt(key, &encrypted).unwrap();
        assert_eq!(decrypted, secret_data);
    }

    mod hex {
        pub fn encode(bytes: &[u8]) -> String {
            bytes.iter().map(|b| format!("{:02x}", b)).collect()
        }

        pub fn decode(hex_str: &str) -> Vec<u8> {
            (0..hex_str.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex_str[i..i + 2], 16).unwrap())
                .collect()
        }
    }
}
