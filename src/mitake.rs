// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Mitake download manifests and their records.

use crate::binary::Reader;
use crate::utils::Platform;
use crate::{decryptor, utils};
use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer};
use std::borrow::Cow;
use std::path::PathBuf;

/// Encryption of a Mitake asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CryptoAlgorithm {
    #[default]
    None,
    Xor,
    Aes,
}

impl<'de> Deserialize<'de> for CryptoAlgorithm {
    /// Accepts both the numeric value and the member name.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Num(i64),
            Str(String),
        }
        match Repr::deserialize(d)? {
            Repr::Num(0) => Ok(Self::None),
            Repr::Num(1) => Ok(Self::Xor),
            Repr::Num(2) => Ok(Self::Aes),
            Repr::Str(s) if s.eq_ignore_ascii_case("none") => Ok(Self::None),
            Repr::Str(s) if s.eq_ignore_ascii_case("xor") => Ok(Self::Xor),
            Repr::Str(s) if s.eq_ignore_ascii_case("aes") => Ok(Self::Aes),
            // unknown numbers: plain copy
            Repr::Num(_) => Ok(Self::None),
            Repr::Str(s) => Err(serde::de::Error::custom(format!(
                "unknown crypto algorithm '{s}'"
            ))),
        }
    }
}

/// Hash of a record, from which its AES key is derived.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct RecordedHash {
    pub u0: u32,
    pub u1: u32,
    pub u2: u32,
    pub u3: u32,
}

impl RecordedHash {
    /// Derives the 256-bit AES key of the record.
    pub fn key256(&self) -> Vec<u8> {
        let (u0, u1, u2, u3) = (self.u0, self.u1, self.u2, self.u3);
        [
            u1 ^ u0,
            u2 ^ u1,
            u3 ^ u2,
            u3 ^ u0,
            u2 ^ u0,
            u3 ^ u1,
            (u0 << 0x10) | (u1 >> 0x10),
            (u2 << 0x10) | (u3 >> 0x10),
        ]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect()
    }
}

/// Asset bundle of a manifest.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AbRecord {
    pub project_name: Option<String>,
    pub asset_bundle_name: Option<String>,
    pub crypto_algorithm: CryptoAlgorithm,
    pub offset_in_bytes: i32,
    pub hash: RecordedHash,
    pub is_streaming_scene_asset_bundle: bool,
    pub size: i64,
    pub last_write_time: i64,
}

impl AbRecord {
    /// Returns the project of the bundle.
    fn project(&self) -> &str {
        self.project_name.as_deref().unwrap_or("")
    }
    /// Returns the name of the bundle.
    fn bundle(&self) -> &str {
        self.asset_bundle_name.as_deref().unwrap_or("")
    }

    /// Returns the CDN path of the bundle on a platform.
    pub fn url(&self, platform: Platform) -> String {
        platform.mitake_path(self.project(), self.bundle())
    }

    /// Returns what tells this version of the bundle from its other ones, for the
    /// record of what is finished.
    pub fn state_key(&self) -> String {
        let RecordedHash { u0, u1, u2, u3 } = self.hash;
        format!(
            "{}/{} {u0} {u1} {u2} {u3} {} {}",
            self.project(),
            self.bundle(),
            self.size,
            self.last_write_time
        )
    }

    /// Returns the output path of the bundle of a platform.
    pub fn path(&self, platform: Platform) -> Result<PathBuf> {
        utils::get_file_path(platform, self.project(), self.bundle())
    }

    /// Strips the header and decrypts a downloaded bundle.
    pub fn unpack<'a>(&self, data: &'a [u8]) -> Result<Cow<'a, [u8]>> {
        let offset = self.offset_in_bytes;
        if offset < 0 || offset as usize > data.len() {
            bail!("header past the end of the bundle");
        }
        let body = &data[offset as usize..];

        Ok(match self.crypto_algorithm {
            CryptoAlgorithm::Xor => bail!("unsupported Xor encryption"),
            CryptoAlgorithm::Aes => {
                let mut r = Reader::new(body);
                let iv_count = r.i32()?;
                let iv = r.read_bytes(iv_count)?;
                r.i32()?;
                decryptor::aes_dec::decrypt(r.remaining(), &self.hash.key256(), iv)?.into()
            }
            CryptoAlgorithm::None => body.into(),
        })
    }
}

/// Download manifest: JSON list of records, inflated.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AbDownloadManifest {
    #[serde(rename = "_records")]
    pub records: Option<Vec<AbRecord>>,
}

impl AbDownloadManifest {
    /// Reads a manifest: its u32 inflated size, then its raw Deflate data.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 4 {
            bail!("manifest too short");
        }
        let size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let json = utils::inflate(&data[4..], size)?;
        // lossy UTF-8
        Ok(serde_json::from_str(&String::from_utf8_lossy(&json))?)
    }
}
