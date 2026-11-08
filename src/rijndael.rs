// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Rijndael with 128, 192 or 256-bit blocks and keys, for CBC decryption.

use std::sync::OnceLock;

/// Block size.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum BlockSize {
    B128,
    B192,
    B256,
}

/// Key size.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum KeySize {
    K128,
    K192,
    K256,
}

/// Largest block (256 bits) in bytes.
const MAX_BLOCK: usize = 32;

/// S-boxes and the multiplication tables of the inverse MixColumns.
struct Tables {
    sbox: [u8; 256],
    inv: [u8; 256],
    m9: [u8; 256],
    m11: [u8; 256],
    m13: [u8; 256],
    m14: [u8; 256],
}

/// Multiplies by 2 in GF(2^8).
fn xtime(a: u8) -> u8 {
    (a << 1) ^ if a & 0x80 != 0 { 0x1b } else { 0 }
}

/// Multiplies two elements of GF(2^8).
fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut p = 0;
    while b != 0 {
        if b & 1 != 0 {
            p ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    p
}

/// Returns the tables, computed on first use.
fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut t = Tables {
            sbox: [0; 256],
            inv: [0; 256],
            m9: [0; 256],
            m11: [0; 256],
            m13: [0; 256],
            m14: [0; 256],
        };
        for x in 0..=255u8 {
            let y = (1..=255u8).find(|&c| gmul(x, c) == 1).unwrap_or(0);
            let s = y
                ^ y.rotate_left(1)
                ^ y.rotate_left(2)
                ^ y.rotate_left(3)
                ^ y.rotate_left(4)
                ^ 0x63;
            t.sbox[x as usize] = s;
            t.inv[s as usize] = x;
            t.m9[x as usize] = gmul(x, 9);
            t.m11[x as usize] = gmul(x, 11);
            t.m13[x as usize] = gmul(x, 13);
            t.m14[x as usize] = gmul(x, 14);
        }
        t
    })
}

/// Cipher with its expanded key.
pub struct Rijndael {
    t: &'static Tables,
    nb: usize,
    nr: usize,
    shifts: [usize; 4],
    round_keys: Vec<u8>, // (nr+1) * nb * 4 bytes
}

impl Rijndael {
    /// Expands a key, zero-padded or truncated to the key size.
    pub fn new(key: &[u8], block: BlockSize, ks: KeySize) -> Self {
        let nb = match block {
            BlockSize::B128 => 4,
            BlockSize::B192 => 6,
            BlockSize::B256 => 8,
        };
        let nk = match ks {
            KeySize::K128 => 4,
            KeySize::K192 => 6,
            KeySize::K256 => 8,
        };
        let nr = nb.max(nk) + 6;
        let t = tables();

        let mut k = vec![0u8; nk * 4];
        let n = key.len().min(k.len());
        k[..n].copy_from_slice(&key[..n]);

        let total = nb * (nr + 1);
        let mut w: Vec<[u8; 4]> = k.as_chunks::<4>().0.to_vec();
        w.reserve(total - nk);
        let mut rcon = 1u8;
        for i in nk..total {
            let mut tmp = w[i - 1];
            if i % nk == 0 {
                tmp = [tmp[1], tmp[2], tmp[3], tmp[0]].map(|b| t.sbox[b as usize]);
                tmp[0] ^= rcon;
                rcon = xtime(rcon);
            } else if nk > 6 && i % nk == 4 {
                tmp = tmp.map(|b| t.sbox[b as usize]);
            }
            let p = w[i - nk];
            w.push([p[0] ^ tmp[0], p[1] ^ tmp[1], p[2] ^ tmp[2], p[3] ^ tmp[3]]);
        }
        let shifts = if nb == 8 { [0, 1, 3, 4] } else { [0, 1, 2, 3] };
        Self {
            t,
            nb,
            nr,
            shifts,
            round_keys: w.into_iter().flatten().collect(),
        }
    }

    /// Returns the block size in bytes.
    pub fn block_bytes(&self) -> usize {
        self.nb * 4
    }

    /// XORs the round key of `round` into a block.
    fn add_round_key(&self, s: &mut [u8], round: usize) {
        let (off, len) = (round * s.len(), s.len());
        for (b, k) in s.iter_mut().zip(&self.round_keys[off..off + len]) {
            *b ^= k;
        }
    }

    /// Shifts the rows of a block right, by 1 to 4 columns.
    fn inv_shift_rows(&self, s: &mut [u8]) {
        let mut old = [0u8; MAX_BLOCK];
        old[..s.len()].copy_from_slice(s);
        for r in 1..4 {
            for c in 0..self.nb {
                s[r + 4 * ((c + self.shifts[r]) % self.nb)] = old[r + 4 * c];
            }
        }
    }

    /// Applies the inverse MixColumns to each column of a block.
    fn inv_mix_columns(&self, s: &mut [u8]) {
        let t = self.t;
        for col in s.as_chunks_mut::<4>().0 {
            let [a, b, c, d] = [
                col[0] as usize,
                col[1] as usize,
                col[2] as usize,
                col[3] as usize,
            ];
            col[0] = t.m14[a] ^ t.m11[b] ^ t.m13[c] ^ t.m9[d];
            col[1] = t.m9[a] ^ t.m14[b] ^ t.m11[c] ^ t.m13[d];
            col[2] = t.m13[a] ^ t.m9[b] ^ t.m14[c] ^ t.m11[d];
            col[3] = t.m11[a] ^ t.m13[b] ^ t.m9[c] ^ t.m14[d];
        }
    }

    /// Applies the inverse S-box to each byte of a block.
    fn inv_sub_bytes(&self, s: &mut [u8]) {
        for b in s.iter_mut() {
            *b = self.t.inv[*b as usize];
        }
    }

    /// Decrypts a block in place.
    pub fn decrypt_block(&self, s: &mut [u8]) {
        self.add_round_key(s, self.nr);
        for round in (1..self.nr).rev() {
            self.inv_shift_rows(s);
            self.inv_sub_bytes(s);
            self.add_round_key(s, round);
            self.inv_mix_columns(s);
        }
        self.inv_shift_rows(s);
        self.inv_sub_bytes(s);
        self.add_round_key(s, 0);
    }

    /// Encrypts a block in place, for the tests.
    #[cfg(test)]
    pub fn encrypt_block(&self, s: &mut [u8]) {
        let nb = self.nb;
        let shifts = self.shifts;
        self.add_round_key(s, 0);
        for round in 1..=self.nr {
            for b in s.iter_mut() {
                *b = self.t.sbox[*b as usize];
            }
            let old = s.to_vec();
            for r in 1..4 {
                for c in 0..nb {
                    s[r + 4 * c] = old[r + 4 * ((c + shifts[r]) % nb)];
                }
            }
            if round != self.nr {
                for c in 0..nb {
                    let a = [s[4 * c], s[4 * c + 1], s[4 * c + 2], s[4 * c + 3]];
                    s[4 * c] = gmul(a[0], 2) ^ gmul(a[1], 3) ^ a[2] ^ a[3];
                    s[4 * c + 1] = a[0] ^ gmul(a[1], 2) ^ gmul(a[2], 3) ^ a[3];
                    s[4 * c + 2] = a[0] ^ a[1] ^ gmul(a[2], 2) ^ gmul(a[3], 3);
                    s[4 * c + 3] = gmul(a[0], 3) ^ a[1] ^ a[2] ^ gmul(a[3], 2);
                }
            }
            self.add_round_key(s, round);
        }
    }
}

/// Decrypts a message in CBC mode, keeping its padding. A short final block is
/// padded with zeros.
pub fn decrypt_cbc_raw(
    message: &[u8],
    key: &[u8],
    iv: &[u8],
    block: BlockSize,
    ks: KeySize,
) -> Vec<u8> {
    if message.is_empty() {
        return Vec::new();
    }
    let cipher = Rijndael::new(key, block, ks);
    let bs = cipher.block_bytes();

    let mut vector = [0u8; MAX_BLOCK];
    let n = iv.len().min(bs);
    vector[..n].copy_from_slice(&iv[..n]);

    let mut out = message.to_vec();
    for chunk in out.chunks_mut(bs) {
        let len = chunk.len();
        let mut data = [0u8; MAX_BLOCK];
        data[..len].copy_from_slice(chunk);
        let block = &mut data[..bs];
        cipher.decrypt_block(block);
        for (d, v) in block.iter_mut().zip(&vector) {
            *d ^= v;
        }
        vector[..len].copy_from_slice(chunk);
        chunk.copy_from_slice(&block[..len]);
    }
    out
}

/// Decrypts a message in CBC mode. Its last byte is a padding length, ignored
/// if not shorter than the message.
pub fn decrypt_cbc(
    message: &[u8],
    key: &[u8],
    iv: &[u8],
    block: BlockSize,
    ks: KeySize,
) -> Vec<u8> {
    let mut out = decrypt_cbc_raw(message, key, iv, block, ks);
    if let Some(&last) = out.last()
        && out.len() > last as usize
    {
        out.truncate(out.len() - last as usize);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes128_fips197_vector() {
        let key: Vec<u8> = (0u8..16).collect();
        let c = Rijndael::new(&key, BlockSize::B128, KeySize::K128);
        let mut b: Vec<u8> = (0u8..16).map(|i| i * 0x11).collect();
        c.encrypt_block(&mut b);
        assert_eq!(hex(&b), "69c4e0d86a7b0430d8cdb78070b4c55a");
        c.decrypt_block(&mut b);
        assert_eq!(hex(&b), "00112233445566778899aabbccddeeff");
    }

    #[test]
    fn aes256_fips197_vector() {
        let key: Vec<u8> = (0u8..32).collect();
        let c = Rijndael::new(&key, BlockSize::B128, KeySize::K256);
        let mut b: Vec<u8> = (0u8..16).map(|i| i * 0x11).collect();
        c.encrypt_block(&mut b);
        assert_eq!(hex(&b), "8ea2b7ca516745bfeafc49904b496089");
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn md5_hex(b: &[u8]) -> String {
        use md5::{Digest, Md5};
        hex(&Md5::digest(b))
    }

    const KEY: &[u8] = b"lrZ6++Ln5tLnBsJk.ae6J8BLaLbMhVn@";
    const IV: &[u8] = b"X6TU@VYU$HyqKy57PfwWg7.t7wk2oqtg";

    // reference values produced by HOMEManager
    #[test]
    fn matches_reference_for_all_block_and_key_sizes() {
        let data: Vec<u8> = (0..0x240usize).map(|i| ((i * 7 + 3) % 256) as u8).collect();
        let cases = [
            (
                BlockSize::B128,
                KeySize::K128,
                443,
                "cfc9dbd32c2cb563c2f51192230f8783",
            ),
            (
                BlockSize::B192,
                KeySize::K256,
                354,
                "70003f469f51ff93cfa0ec52a760b25a",
            ),
            (
                BlockSize::B256,
                KeySize::K128,
                338,
                "6c3bec513a4019d3ca22e3785372a5f0",
            ),
            (
                BlockSize::B256,
                KeySize::K192,
                522,
                "b24a57571c9b28f77fa913f9338f4904",
            ),
        ];
        for (b, k, len, hash) in cases {
            let out = decrypt_cbc(&data, KEY, IV, b, k);
            assert_eq!(out.len(), len);
            assert_eq!(md5_hex(&out), hash);
        }
    }

    #[test]
    fn aba_decrypt_matches_reference() {
        let data: Vec<u8> = (0..0x420usize).map(|i| ((i * 7 + 3) % 256) as u8).collect();
        let out = crate::decryptor::aba::decrypt(&data).unwrap();
        assert_eq!(out.len(), 963);
        assert_eq!(md5_hex(&out), "a1827f19023c219bf993796f20d4776e");
    }
}
