// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Asset bundle pack list and packs.

use crate::binary::Reader;
use crate::state::WrittenFile;
use crate::utils::Platform;
use crate::{decryptor, utils};
use anyhow::{Result, bail};

/// Entry of the `MD_AssetbundleDLPackVer.snd` list.
pub struct PackListEntry {
    pub nm: String,
    /// Version of the pack on the platform of the list read.
    pub version: i32,
}

impl PackListEntry {
    /// Reads an entry: its name, versions and file list.
    fn parse(r: &mut Reader, platform: Platform) -> Result<Self> {
        let nm = r.dotnet_string()?;
        r.bytes(8)?; // sa, si
        let vr_ios = r.i32()?;
        let vr_dro = r.i32()?;
        // file list, after a count string such as "#3"
        let count = r.dotnet_string()?;
        let mut chars = count.chars();
        if chars.next().is_none() {
            bail!("empty file count in the pack list");
        }
        for _ in 0..chars.as_str().trim().parse::<i32>().unwrap_or(0) {
            r.dotnet_string()?;
        }
        let version = platform.pack_version(vr_ios, vr_dro);
        Ok(Self { nm, version })
    }
}

/// Reads the asset bundle pack list, keeping the versions of `platform`.
pub fn read_md(bytes: &[u8], platform: Platform) -> Result<Vec<PackListEntry>> {
    let mut r = Reader::new(bytes);
    r.i32()?;
    let count = r.i32()?;
    r.i32()?;
    r.i32()?;
    if count < 0 {
        bail!("negative entry count in the pack list");
    }
    (0..count)
        .map(|_| PackListEntry::parse(&mut r, platform))
        .collect()
}

/// Signature of the asset bundle packs.
const SIGNATURE: &str = "ppir";

/// Decrypts each entry of an asset bundle pack to
/// `<output>/<platform>/<dir>/<entry>.unity3d`. Returns the files it holds.
pub fn parse_abap(data: &[u8], dir: &str, platform: Platform) -> Result<Vec<WrittenFile>> {
    let mut r = Reader::new(data);
    if r.string_to_null(4) != SIGNATURE {
        bail!("not an asset bundle pack");
    }

    let root_head_offset = r.u32()?;
    let name_head_offset = r.u32()? as i64;
    let data_head_offset = r.u32()? as i64;
    let root_buffer_size = (name_head_offset as u32).wrapping_sub(root_head_offset);
    let count = ((root_buffer_size as i32) >> 4).max(0);

    // (name_offset, name_size, data_offset, data_size) per entry
    let infos = (0..count)
        .map(|_| Ok([r.i32()?, r.i32()?, r.i32()?, r.i32()?]))
        .collect::<Result<Vec<_>>>()?;

    let mut files = Vec::new();
    for [name_offset, name_size, data_offset, data_size] in infos {
        r.seek((name_head_offset + name_offset as i64).max(0) as usize);
        let name = r.string_to_null(name_size.max(0) as usize);

        r.seek((data_head_offset + data_offset as i64).max(0) as usize);
        let entry_data = r.read_bytes(data_size)?;

        let decrypted = decryptor::aba::decrypt(entry_data)?;
        let path = utils::get_file_path(platform, dir, &name)?;
        if utils::write_if_changed(&path, &decrypted)? {
            crate::ui::file_written();
        }
        files.push((path, decrypted.len() as u64));
    }
    Ok(files)
}
