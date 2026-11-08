// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Paths, hashes and file helpers.

use anyhow::{Result, bail};
use flate2::read::DeflateDecoder;
use md5::{Digest, Md5};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const CDN: &str = "ph.prd.cdnfiles";
const MD: &str = "MD_AssetbundleDLPackVer";
const PN: &str = "MitakeCommon";

/// Lowest MDVR probed by the detection, a version known to exist.
pub const MIN_MDVR: i32 = 70900;

/// Directory the bundles are written to.
static OUTPUT: OnceLock<PathBuf> = OnceLock::new();

/// Release of the game the bundles are built for.
#[derive(Clone, Copy)]
pub enum Platform {
    Android,
    Ios,
}

impl Platform {
    /// Returns the directory of the platform in the output directory.
    pub fn output_dir(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }

    /// Returns the name shown for the asset bundle packs of the platform.
    pub fn pack_source(self) -> &'static str {
        match self {
            Self::Android => "aba",
            Self::Ios => "abi",
        }
    }

    /// Returns the name shown for the Mitake asset bundles of the platform.
    pub fn mitake_source(self) -> &'static str {
        match self {
            Self::Android => "mitake-android",
            Self::Ios => "mitake-ios",
        }
    }

    /// Returns the version of a pack on the platform, among those of the pack
    /// list.
    pub fn pack_version(self, vr_ios: i32, vr_dro: i32) -> i32 {
        match self {
            Self::Android => vr_dro,
            Self::Ios => vr_ios,
        }
    }

    /// Returns the CDN path of the asset bundle pack list of an MDVR.
    pub fn md_path(self, mdvr: i32) -> String {
        let dir = match self {
            Self::Android => "dro",
            Self::Ios => "ios",
        };
        format!("{}/md/{dir}/diff/{MD}.snd", calculate_file_hash(mdvr))
    }

    /// Returns the CDN path of an asset bundle pack.
    pub fn pack_path(self, id: i32, name: &str) -> String {
        let (dir, ext) = match self {
            Self::Android => ("dro", "abap"),
            Self::Ios => ("ios", "abip"),
        };
        format!("{}/{dir}/{name}.{ext}", calculate_file_hash(id))
    }

    /// Returns the CDN root of the Mitake asset bundles.
    fn mitake_root(self) -> &'static str {
        match self {
            Self::Android => "Models/Android",
            Self::Ios => "Models/iOS",
        }
    }

    /// Returns the CDN path of the Mitake root manifest.
    pub fn mitake_manifest_path(self) -> String {
        format!("{}/{PN}.bin", self.mitake_root())
    }

    /// Returns the file name of the Mitake root manifest.
    pub fn mitake_manifest_name(self) -> String {
        format!("{PN}.bin")
    }

    /// Returns the CDN path of a Mitake asset bundle.
    pub fn mitake_path(self, project: &str, name: &str) -> String {
        combine(&[self.mitake_root(), project, name])
    }
}

/// Writes `data` unless `path` holds it already. Returns true if written.
pub fn write_if_changed(path: impl AsRef<Path>, data: &[u8]) -> Result<bool> {
    let path = path.as_ref();
    // different sizes cannot match: no need to read
    let same_len = std::fs::metadata(path).is_ok_and(|m| m.len() == data.len() as u64);
    if same_len && std::fs::read(path).is_ok_and(|old| old == data) {
        return Ok(false);
    }

    // through a temporary file, so that an interruption leaves no truncated file
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let result = std::fs::write(&temporary, data).and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(true)
}

/// Sets the directory the bundles are written to, before any is written.
pub fn set_output_dir(dir: PathBuf) {
    let _ = OUTPUT.set(dir);
}

/// Returns the directory the bundles are written to, `bundles` in the current
/// directory unless set.
pub fn output_dir() -> &'static Path {
    OUTPUT.get_or_init(|| PathBuf::from("bundles"))
}

/// Joins path parts: empty parts are skipped, and an absolute part starts over.
pub fn combine(parts: &[&str]) -> String {
    let mut out = String::new();
    for p in parts {
        if p.is_empty() {
            continue;
        }
        if out.is_empty() || p.starts_with('/') {
            out = (*p).to_string();
        } else {
            if !out.ends_with('/') {
                out.push('/');
            }
            out.push_str(p);
        }
    }
    out
}

/// Returns the file name of a path without its extension.
pub fn file_name_without_extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or("");
    match name.rfind('.') {
        Some(i) => name[..i].to_string(),
        None => name.to_string(),
    }
}

/// Returns the file name of the asset bundle pack list of an MDVR, which tells
/// it from those of the other MDVRs, all named the same on the CDN.
pub fn md_file_name(mdvr: i32) -> String {
    format!("{MD}_{mdvr}.snd")
}

/// Returns the record of what is finished for a platform, `<name>` telling its
/// downloads apart, kept away from the bundles.
pub fn state_path(platform: Platform, name: &str) -> PathBuf {
    output_dir()
        .join(".state")
        .join(format!("{}-{name}", platform.output_dir()))
}

/// Writes a file listing to `<output>/<platform>/<name>`, unless unchanged.
/// Returns true if written.
pub fn write_listing(platform: Platform, name: &str, data: &[u8]) -> Result<bool> {
    let dir = output_dir().join(platform.output_dir());
    std::fs::create_dir_all(&dir)?;
    write_if_changed(dir.join(name), data)
}

/// Returns `<output>/<platform>/<bundle>/<name>.unity3d`, creating its parent
/// directory.
pub fn get_file_path(platform: Platform, bundle: &str, name: &str) -> Result<PathBuf> {
    // names come from remote data: anything leaving the output directory is refused
    for part in [bundle, name] {
        if part.starts_with('/')
            || part.starts_with('\\')
            || part.split(['/', '\\']).any(|c| c == "..")
        {
            bail!("unsafe path '{part}'");
        }
    }
    let out = output_dir().to_string_lossy().into_owned();
    let path = PathBuf::from(combine(&[
        &out,
        platform.output_dir(),
        bundle,
        &format!("{name}.unity3d"),
    ]));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(path)
}

/// Returns the hashed CDN directory of an id: the MD5 digest of the hexadecimal
/// MD5 digest of "ph.prd.cdnfiles/<id>", rotated left by its first digit.
pub fn calculate_file_hash(id: i32) -> String {
    let hash = md5_hex(&format!("{CDN}/{id}"));
    let idx = hash
        .chars()
        .next()
        .and_then(|c| c.to_digit(16))
        .unwrap_or(0) as usize;
    let rotated = format!("{}{}", &hash[idx..], &hash[..idx]);
    md5_hex(&rotated)
}

/// Returns the MD5 digest of a string, in lowercase hexadecimal.
fn md5_hex(value: &str) -> String {
    use std::fmt::Write;
    Md5::digest(value.as_bytes())
        .iter()
        .fold(String::with_capacity(32), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Inflates raw Deflate data to exactly `inflated_size` bytes.
pub fn inflate(data: &[u8], inflated_size: u32) -> Result<Vec<u8>> {
    let size = inflated_size as usize;
    // the size comes from remote data: it is not trusted for the allocation
    let mut out = Vec::with_capacity(size.min(data.len().saturating_mul(16)));
    DeflateDecoder::new(data)
        .take(inflated_size as u64)
        .read_to_end(&mut out)?;
    if out.len() != size {
        bail!(
            "data inflated to {} bytes instead of {inflated_size}",
            out.len()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_skips_empty_and_absolute_parts() {
        assert_eq!(
            combine(&["Models/Android", "p", "a/b"]),
            "Models/Android/p/a/b"
        );
        assert_eq!(combine(&["a/", "b"]), "a/b");
        assert_eq!(combine(&["a", "/b"]), "/b");
        assert_eq!(combine(&["a", "", "b"]), "a/b");
    }

    #[test]
    fn platform_paths() {
        // both packs exist on the CDN
        assert_eq!(
            Platform::Ios.pack_path(69600, "image_seal_p"),
            "0f710fdcc5a258d13fb807dea9b15812/ios/image_seal_p.abip"
        );
        assert_eq!(
            Platform::Android.pack_path(67900, "sounds_bgm"),
            "8ff8757642724dae5fd5cacb881d6639/dro/sounds_bgm.abap"
        );
        assert_eq!(
            Platform::Ios.md_path(71000),
            "99abe4d0794ff42798898076ec42f90e/md/ios/diff/MD_AssetbundleDLPackVer.snd"
        );
        assert_eq!(
            Platform::Android.md_path(71000),
            "99abe4d0794ff42798898076ec42f90e/md/dro/diff/MD_AssetbundleDLPackVer.snd"
        );
        assert_eq!(
            Platform::Android.mitake_manifest_path(),
            "Models/Android/MitakeCommon.bin"
        );
        assert_eq!(Platform::Ios.mitake_path("p", "a/b"), "Models/iOS/p/a/b");
        assert_eq!(md_file_name(71000), "MD_AssetbundleDLPackVer_71000.snd");
        assert_eq!(Platform::Ios.pack_version(1, 2), 1);
        assert_eq!(Platform::Android.pack_version(1, 2), 2);
    }

    #[test]
    fn stem() {
        assert_eq!(file_name_without_extension("x/y/a.b.unity3d"), "a.b");
        assert_eq!(file_name_without_extension("noext"), "noext");
    }

    #[test]
    fn hash_matches_reference() {
        assert_eq!(
            calculate_file_hash(70200),
            "1f4d2a5c869b54a9b6f91f54e692d3c8"
        );
        assert_eq!(
            calculate_file_hash(70900),
            "d63ed8cc7574c33d48574fe64ae5b6c1"
        );
        assert_eq!(calculate_file_hash(1), "26253abf1fe60d821ea9b42ef0b848b3");
        assert_eq!(calculate_file_hash(0), "d6ee52a62814074c3995ce71f6e09f9d");
    }

    #[test]
    fn write_if_changed_skips_identical() {
        let p = std::env::temp_dir().join("megaturtle_dl_write_test");
        let _ = std::fs::remove_file(&p);
        assert!(write_if_changed(&p, b"a").unwrap());
        assert!(!write_if_changed(&p, b"a").unwrap());
        assert!(write_if_changed(&p, b"b").unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), b"b");
        assert!(
            !std::env::temp_dir()
                .join("megaturtle_dl_write_test.tmp")
                .exists()
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn hash_is_32_hex() {
        let h = calculate_file_hash(70200);
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
