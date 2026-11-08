// SPDX-FileCopyrightText: 2016 Radu
// SPDX-FileCopyrightText: 2016-2022 Perfare
// SPDX-FileCopyrightText: 2021-2026 aelurum
// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Minimal Unity reader, finding the nested manifests of the Mitake asset
//! bundles: UnityFS bundles of header version 7 or later, whose blocks are stored
//! or LZ4 compressed, holding serialized files of format 22 or later, with
//! `TextAsset` objects.
//! Derived from AssetStudioMod by aelurum:
//! https://github.com/aelurum/AssetStudioMod

use crate::binary::Reader;
use anyhow::{Context, Result, anyhow, bail};
use std::borrow::Cow;
use std::ops::Range;

const CLASS_TEXT_ASSET: i32 = 49;

/// Path and range in the data stream of each node of a bundle.
type Nodes = Vec<(String, Range<usize>)>;

/// Object of a serialized file.
struct ObjectInfo {
    class_id: i32,
    range: Range<usize>,
}

/// Serialized file, with its objects.
struct SerializedFile<'a> {
    data: &'a [u8],
    little_endian: bool,
    objects: Vec<ObjectInfo>,
}

// bundle (UnityFS)

/// Decompresses a block of a bundle to `size` bytes, `kind` being its
/// compression.
fn decompress(kind: u32, src: &[u8], size: usize) -> Result<Cow<'_, [u8]>> {
    match kind {
        0 => Ok(src.into()),
        2 | 3 => lz4_flex::block::decompress(src, size)
            .map(Cow::Owned)
            .map_err(|e| anyhow!("LZ4 decompression failed: {e}")),
        _ => bail!("unsupported compression type {kind}"),
    }
}

/// Reads a UnityFS bundle. Returns its decompressed data stream and its nodes.
fn read_bundle(data: &[u8]) -> Result<(Vec<u8>, Nodes)> {
    let mut r = Reader::big_endian(data);
    let signature = r.cstring()?;
    let version = r.u32()?;
    r.cstring()?; // player version
    r.cstring()?; // Unity version
    if signature != "UnityFS" || version < 7 {
        bail!("unsupported bundle '{signature}' version {version}");
    }
    r.i64()?; // size
    let compressed_info_size = r.u32()? as usize;
    let info_size = r.u32()? as usize;
    let flags = r.u32()?;

    r.align(16);

    let info = if flags & 0x80 != 0 {
        // blocks info at the end of the file
        let start = data
            .len()
            .checked_sub(compressed_info_size)
            .context("invalid blocks info size")?;
        &data[start..]
    } else {
        r.bytes(compressed_info_size)?
    };
    let info = decompress(flags & 0x3F, info, info_size)?;

    let mut b = Reader::big_endian(&info);
    b.bytes(16)?; // hash
    let blocks = (0..b.i32()?.max(0))
        .map(|_| {
            Ok((
                b.u32()? as usize,
                b.u32()? as usize,
                u32::from(b.u16()? & 0x3F),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut raw_nodes = Vec::new();
    for _ in 0..b.i32()?.max(0) {
        let offset = b.i64()? as usize;
        let size = b.i64()? as usize;
        b.u32()?; // flags
        raw_nodes.push((b.cstring()?, offset, size));
    }

    // flag 0x200: data blocks are 16-byte aligned
    if flags & 0x200 != 0 {
        r.align(16);
    }
    let total: usize = blocks.iter().map(|b| b.0).sum();
    let mut stream = Vec::with_capacity(total.min(data.len().saturating_mul(64)));
    for (size, compressed, kind) in blocks {
        stream.extend_from_slice(&decompress(kind, r.bytes(compressed)?, size)?);
    }

    let nodes = raw_nodes
        .into_iter()
        .map(|(path, start, size)| {
            let end = start.checked_add(size).filter(|&e| e <= stream.len());
            Ok((path, start..end.context("bundle node outside of the data")?))
        })
        .collect::<Result<_>>()?;
    Ok((stream, nodes))
}

// serialized file

/// Skips a type of a serialized file. Returns its class id.
fn skip_serialized_type(r: &mut Reader, type_tree: bool) -> Result<i32> {
    let class_id = r.i32()?;
    r.bool()?; // is stripped
    r.i16()?; // script type index
    if class_id == 114 {
        r.bytes(16)?; // script id
    }
    r.bytes(16)?; // old type hash
    if type_tree {
        let nodes = r.i32()?.max(0) as usize;
        let strings = r.i32()?.max(0) as usize;
        r.bytes(
            nodes
                .checked_mul(32)
                .and_then(|n| n.checked_add(strings))
                .context("invalid type tree")?,
        )?;
        let n = r.i32()?.max(0) as usize;
        r.bytes(n * 4)?; // dependencies
    }
    Ok(class_id)
}

/// Reads the header, types and object table of a serialized file.
fn read_serialized_file(data: &[u8]) -> Result<SerializedFile<'_>> {
    let mut r = Reader::big_endian(data);
    r.u32()?; // metadata size
    r.u32()?; // file size
    let version = r.u32()?;
    r.u32()?; // data offset, moved below
    if !(22..=40).contains(&version) {
        bail!("unsupported serialized file version {version}");
    }
    let little_endian = r.u8()? == 0;
    r.bytes(3)?;
    r.u32()?; // metadata size
    r.i64()?; // file size
    let data_offset = r.i64()? as u64;
    r.i64()?;
    r.little_endian = little_endian;

    r.cstring()?; // Unity version
    r.i32()?; // target platform
    let type_tree = r.bool()?;

    let class_ids = (0..r.i32()?.max(0))
        .map(|_| skip_serialized_type(&mut r, type_tree))
        .collect::<Result<Vec<_>>>()?;

    let mut objects = Vec::new();
    for _ in 0..r.i32()?.max(0) {
        r.align(4);
        r.i64()?; // path id
        let start = (r.i64()? as u64 + data_offset) as usize;
        let size = r.u32()?;
        let type_id = r.i32()?;
        let class_id = *class_ids
            .get(type_id as usize)
            .context("object of an unknown type")?;
        let range = start..start.saturating_add(size as usize);
        objects.push(ObjectInfo { class_id, range });
    }
    Ok(SerializedFile {
        data,
        little_endian,
        objects,
    })
}

/// Returns a reader of the data of an object.
fn object_reader<'a>(file: &SerializedFile<'a>, obj: &ObjectInfo) -> Result<Reader<'a>> {
    let mut r = Reader::new(
        file.data
            .get(obj.range.clone())
            .context("object outside of its file")?,
    );
    r.little_endian = file.little_endian;
    Ok(r)
}

/// Reads a length-prefixed string, then skips to a 4-byte boundary.
fn aligned_string(r: &mut Reader) -> Result<String> {
    let len = r.i32()?.max(0) as usize;
    let s = String::from_utf8_lossy(r.bytes(len)?).into_owned();
    r.align(4);
    Ok(s)
}

/// Reads a `TextAsset`. Returns its name and its script.
fn read_text_asset<'a>(file: &SerializedFile<'a>, obj: &ObjectInfo) -> Result<(String, &'a [u8])> {
    let mut r = object_reader(file, obj)?;
    let name = aligned_string(&mut r)?;
    let len = r.i32()?;
    if len < 0 {
        bail!("invalid TextAsset script size");
    }
    Ok((name, r.bytes(len as usize)?))
}

/// Checks whether a node holds resources rather than a serialized file.
fn is_resource_node(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".ress") || p.ends_with(".resource")
}

/// Returns the script of the manifest held by a bundle of exactly two objects,
/// one a `TextAsset` named like `stem`, ignoring case, or nothing for any other
/// bundle. A bundle that cannot be read is an error, as it may be a manifest.
pub fn manifest(data: &[u8], stem: &str) -> Result<Option<Vec<u8>>> {
    let (stream, nodes) = read_bundle(data)?;
    let files = nodes
        .iter()
        .filter(|(path, _)| !is_resource_node(path))
        .map(|(_, range)| read_serialized_file(&stream[range.clone()]))
        .collect::<Result<Vec<_>>>()?;
    if files.iter().map(|f| f.objects.len()).sum::<usize>() != 2 {
        return Ok(None);
    }
    let text_asset = files.iter().find_map(|f| {
        f.objects
            .iter()
            .find(|o| o.class_id == CLASS_TEXT_ASSET)
            .map(|o| (f, o))
    });
    let Some((file, obj)) = text_asset else {
        return Ok(None);
    };
    let (name, script) = read_text_asset(file, obj)?;
    let upper = |s: &str| s.chars().flat_map(char::to_uppercase).collect::<String>();
    Ok((upper(&name) == upper(stem)).then(|| script.to_vec()))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn be32(v: &mut Vec<u8>, x: u32) {
        v.extend(x.to_be_bytes());
    }

    /// Builds a tiny serialized file (format 22, little endian, no type tree)
    /// holding an AssetBundle object and a TextAsset.
    pub fn build_serialized(name: &str, script: &[u8]) -> Vec<u8> {
        // object payloads
        let mut ta = Vec::new();
        ta.extend((name.len() as i32).to_le_bytes());
        ta.extend(name.as_bytes());
        while ta.len() % 4 != 0 {
            ta.push(0);
        }
        ta.extend((script.len() as i32).to_le_bytes());
        ta.extend(script);
        let ab = vec![0u8; 8];

        let mut meta = Vec::new();
        meta.extend(b"2022.2.2f1\0");
        meta.extend(13i32.to_le_bytes()); // platform
        meta.push(0); // enable type tree = false
        meta.extend(2i32.to_le_bytes()); // type count
        for class_id in [142i32, 49] {
            meta.extend(class_id.to_le_bytes());
            meta.push(0); // stripped
            meta.extend((-1i16).to_le_bytes());
            meta.extend([0u8; 16]); // old type hash
        }
        meta.extend(2i32.to_le_bytes()); // object count
        let ta_off = (ab.len() as u64 + 7) & !7;
        let objs = [
            (1i64, 0u64, ab.len() as u32, 0i32),
            (2, ta_off, ta.len() as u32, 1),
        ];
        for (pid, off, size, tid) in objs {
            while meta.len() % 4 != 0 {
                meta.push(0);
            }
            meta.extend(pid.to_le_bytes());
            meta.extend(off.to_le_bytes());
            meta.extend(size.to_le_bytes());
            meta.extend(tid.to_le_bytes());
        }

        // header (48 bytes, a multiple of 4: the alignment above holds)
        let header_len = 48usize;
        let data_offset = (header_len + meta.len() + 15) & !15;
        let file_size = data_offset + ta_off as usize + ta.len();
        let mut file = Vec::new();
        be32(&mut file, 0);
        be32(&mut file, 0);
        be32(&mut file, 22);
        be32(&mut file, 0);
        file.extend([0u8, 0, 0, 0]);
        be32(&mut file, meta.len() as u32);
        file.extend((file_size as i64).to_be_bytes());
        file.extend((data_offset as i64).to_be_bytes());
        file.extend(0i64.to_be_bytes());
        file.extend(&meta);
        file.resize(data_offset, 0);
        file.extend(&ab);
        file.resize(data_offset + ta_off as usize, 0);
        file.extend(&ta);
        file
    }

    /// Wraps data as a UnityFS bundle with one LZ4 block.
    pub fn build_bundle(node_name: &str, payload: &[u8]) -> Vec<u8> {
        let comp = lz4_flex::block::compress(payload);
        let mut info = Vec::new();
        info.extend([0u8; 16]);
        be32(&mut info, 1);
        be32(&mut info, payload.len() as u32);
        be32(&mut info, comp.len() as u32);
        info.extend(3u16.to_be_bytes()); // LZ4HC
        be32(&mut info, 1);
        info.extend(0i64.to_be_bytes());
        info.extend((payload.len() as i64).to_be_bytes());
        be32(&mut info, 4);
        info.extend(node_name.as_bytes());
        info.push(0);
        let info_comp = lz4_flex::block::compress(&info);

        let mut out = Vec::new();
        out.extend(b"UnityFS\0");
        be32(&mut out, 8);
        out.extend(b"5.x.x\0");
        out.extend(b"2022.2.2f1\0");
        out.extend(0i64.to_be_bytes());
        be32(&mut out, info_comp.len() as u32);
        be32(&mut out, info.len() as u32);
        be32(&mut out, 3 | 0x40);
        // the blocks info is aligned to 16 bytes
        while out.len() % 16 != 0 {
            out.push(0);
        }
        out.extend(&info_comp);
        out.extend(&comp);
        out
    }

    #[test]
    fn reads_manifest_from_bundle() {
        let bundle = build_bundle("CAB-1", &build_serialized("mymanifest", b"hello manifest"));
        assert_eq!(
            manifest(&bundle, "MyManifest").unwrap().as_deref(),
            Some(&b"hello manifest"[..])
        );
        assert_eq!(manifest(&bundle, "Other").unwrap(), None);
    }

    #[test]
    fn unreadable_bundle_is_an_error() {
        assert!(manifest(b"not a bundle\0", "x").is_err());

        // the blocks info compressed with LZMA, not supported: its kind is in
        // the header flags, the last byte before the padding to 16 bytes
        let mut bundle = build_bundle("CAB-1", &build_serialized("x", b"y"));
        let flags = bundle.iter().position(|&b| b == 0x43).unwrap();
        bundle[flags] = 0x41;
        let error = manifest(&bundle, "x").unwrap_err().to_string();
        assert!(error.contains("unsupported compression type 1"), "{error}");
    }
}
