// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Decryption of the asset bundles.

use crate::rijndael::{self, BlockSize, KeySize};
use anyhow::{Result, bail};

/// Entries of the asset bundle packs: Rijndael-256 CBC over their first 0x420
/// bytes.
pub mod aba {
    use super::*;

    const PADDED_SIZE: usize = 0x420;

    const KEY: &[u8] = b"lrZ6++Ln5tLnBsJk.ae6J8BLaLbMhVn@";
    const IV: &[u8] = b"X6TU@VYU$HyqKy57PfwWg7.t7wk2oqtg";

    /// Decrypts an entry.
    pub fn decrypt(data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < PADDED_SIZE {
            bail!(
                "encrypted entry of {} bytes, {PADDED_SIZE} at least",
                data.len()
            );
        }
        let mut out = rijndael::decrypt_cbc(
            &data[..PADDED_SIZE],
            KEY,
            IV,
            BlockSize::B256,
            KeySize::K256,
        );
        out.extend_from_slice(&data[PADDED_SIZE..]);
        Ok(out)
    }
}

/// Mitake asset bundles: AES-256-CBC with PKCS7 padding over their first 0x410
/// bytes.
pub mod aes_dec {
    use super::*;

    const PADDED_SIZE: usize = 0x410;

    const BLOCK_SIZE: usize = 16;

    /// Decrypts a bundle with its key and IV.
    pub fn decrypt(data: &[u8], key: &[u8], iv: &[u8]) -> Result<Vec<u8>> {
        if data.len() < PADDED_SIZE {
            bail!(
                "encrypted bundle of {} bytes, {PADDED_SIZE} at least",
                data.len()
            );
        }
        if key.len() != 32 || iv.len() != BLOCK_SIZE {
            bail!("invalid AES key or IV");
        }
        let mut out = rijndael::decrypt_cbc_raw(
            &data[..PADDED_SIZE],
            key,
            iv,
            BlockSize::B128,
            KeySize::K256,
        );

        // PKCS#7: as many bytes as padded, all of that value
        let padding = usize::from(out[PADDED_SIZE - 1]);
        let start = PADDED_SIZE.wrapping_sub(padding);
        if !(1..=BLOCK_SIZE).contains(&padding)
            || out[start..].iter().any(|&b| usize::from(b) != padding)
        {
            bail!("invalid AES padding");
        }
        out.truncate(start);

        out.extend_from_slice(&data[PADDED_SIZE..]);
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::rijndael::Rijndael;

        /// Encrypts `plain`, of whole blocks, in CBC mode.
        fn encrypt(plain: &[u8], key: &[u8], iv: &[u8]) -> Vec<u8> {
            let cipher = Rijndael::new(key, BlockSize::B128, KeySize::K256);
            let mut previous = iv.to_vec();
            let mut out = Vec::new();
            for block in plain.chunks(BLOCK_SIZE) {
                let mut b: Vec<u8> = block.iter().zip(&previous).map(|(x, y)| x ^ y).collect();
                cipher.encrypt_block(&mut b);
                previous.clone_from(&b);
                out.extend(b);
            }
            out
        }

        fn padded(pad: u8) -> Vec<u8> {
            let mut plain = vec![0xAB; PADDED_SIZE - usize::from(pad)];
            plain.resize(PADDED_SIZE, pad);
            plain
        }

        #[test]
        fn removes_the_padding_and_keeps_the_rest() {
            let (key, iv) = ([1u8; 32], [2u8; BLOCK_SIZE]);
            let mut data = encrypt(&padded(5), &key, &iv);
            data.extend(b"rest");
            let out = decrypt(&data, &key, &iv).unwrap();
            assert_eq!(out.len(), PADDED_SIZE - 5 + 4);
            assert!(out[..PADDED_SIZE - 5].iter().all(|&b| b == 0xAB));
            assert_eq!(&out[PADDED_SIZE - 5..], b"rest");
        }

        #[test]
        fn refuses_bad_input() {
            let (key, iv) = ([1u8; 32], [2u8; BLOCK_SIZE]);
            let data = encrypt(&padded(5), &key, &iv);
            assert!(decrypt(&data[..16], &key, &iv).is_err());
            assert!(decrypt(&data, &key[..16], &iv).is_err());
            assert!(decrypt(&data, &key, &iv[..8]).is_err());
            // a padding length that is out of range, or bytes that do not match it
            for pad in [0, 17] {
                let bad = encrypt(&padded(pad), &key, &iv);
                assert!(decrypt(&bad, &key, &iv).is_err());
            }
            let mut plain = padded(5);
            plain[PADDED_SIZE - 3] = 4;
            assert!(decrypt(&encrypt(&plain, &key, &iv), &key, &iv).is_err());
        }
    }
}
