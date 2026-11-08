<!--
SPDX-FileCopyrightText: 2026 SombrAbsol

SPDX-License-Identifier: MIT
-->

# Pokémon HOME Asset Distribution
Used in *Pokémon HOME* to download its assets from two content delivery networks (CDN): the asset bundle packs (ABA on Android, ABI on iOS), and the Mitake asset bundles.

Please note that this documentation was written based on the code that served as the basis for megaturtle-dl and may therefore be inaccurate or incomplete.

## Table of contents
* [CDN paths](#cdn-paths)
  * [Hashed directories](#hashed-directories)
  * [Asset bundle packs](#asset-bundle-packs)
  * [Mitake asset bundles](#mitake-asset-bundles)
* [Pack list](#pack-list)
* [Asset bundle pack](#asset-bundle-pack)
  * [Entry decryption](#entry-decryption)
* [Mitake manifest](#mitake-manifest)
  * [Records](#records)
  * [Nested manifests](#nested-manifests)
* [Mitake asset bundle](#mitake-asset-bundle)
  * [AES key derivation](#aes-key-derivation)

## CDN paths
### Hashed directories
The files of an asset bundle pack version are stored under a directory named after a hash of that version number `id`:
1. Compute the lowercase hexadecimal MD5 digest of the string `ph.prd.cdnfiles/<id>`
2. Rotate this string to the left by the value of its first hexadecimal digit (0 to 15 characters)
3. The directory name is the lowercase hexadecimal MD5 digest of the rotated string

### Asset bundle packs
Root: `https://resource.pokemon-home.com`
| File | Android | iOS |
|---|---|---|
| Pack list of the master data version (MDVR) `mdvr` | `<hash(mdvr)>/md/dro/diff/MD_AssetbundleDLPackVer.snd` | `<hash(mdvr)>/md/ios/diff/MD_AssetbundleDLPackVer.snd` |
| Asset bundle pack named `nm` | `<hash(vr_dro)>/dro/<nm>.abap` | `<hash(vr_ios)>/ios/<nm>.abip` |

Both platforms use the same formats and keys, but their packs differ, each holding the Unity asset bundles built for its platform. The pack list of an MDVR is the same on both.

The latest MDVR is the highest multiple of 100, from 70900 on, whose pack list exists on the CDN. It is searched up to 10000 above the highest one found.

Older versions remain on the CDN, the oldest found being 35700. They are all multiples of 100, as far as probing the ranges around some of them shows.

### Mitake asset bundles
Root: `https://d2gf339i9nrwgc.cloudfront.net`
* Root manifest: `Models/<platform>/MitakeCommon.bin`
* Asset bundle: `Models/<platform>/<projectName>/<assetBundleName>`

`<platform>` is `Android` or `iOS`.

## Pack list
* 32-bit little-endian integer values
* Strings are .NET strings: a 7-bit encoded length (7 bits per byte, least significant first, the high bit telling that another byte follows), followed by as many bytes of UTF-8

```rust
{
  i32 unknown1
  i32 count     // number of entries
  i32 unknown2
  i32 unknown3
}
```

Immediately followed by `count` entries:
```rust
{
  string nm         // pack name
  i32    sa         // size of the Android pack
  i32    si         // size of the iOS pack
  i32    vr_ios     // version of the iOS pack
  i32    vr_dro     // version of the Android pack
  string file_count // "#" followed by the number of file names
  string files[]    // file names of the pack
}
```

## Asset bundle pack
```rust
{
  char[4] signature        // "ppir"
  u32     root_head_offset // start of the entry table
  u32     name_head_offset // start of the entry names
  u32     data_head_offset // start of the entry data
}
```

Immediately followed by `(name_head_offset - root_head_offset) / 16` entries:
```rust
{
  i32 name_offset // relative to name_head_offset
  i32 name_size   // in characters, the name ending earlier at a null character
  i32 data_offset // relative to data_head_offset
  i32 data_size
}
```

### Entry decryption
Each entry is an encrypted UnityFS asset bundle:
* The first `0x420` bytes are encrypted with Rijndael in CBC mode, with 256-bit blocks and a 256-bit key:
  * Key: `lrZ6++Ln5tLnBsJk.ae6J8BLaLbMhVn@`
  * IV: `X6TU@VYU$HyqKy57PfwWg7.t7wk2oqtg`
* Once decrypted, the last byte of these `0x420` bytes is a padding length, removed if it is shorter than them
* The rest of the entry is stored as is

## Mitake manifest
```rust
{
  u32 size   // little-endian size of the inflated data
  u8  data[] // raw Deflate data
}
```

Once inflated, the data is a UTF-8 JSON object, whose `_records` member is an array of records.

### Records
```rust
{
  string projectName
  string assetBundleName
  int    cryptoAlgorithm             // 0 or "None", 1 or "Xor", 2 or "Aes"
  int    offsetInBytes               // size of the header of the bundle
  object hash                        // { u0, u1, u2, u3 }, 32-bit integers
  bool   isStreamingSceneAssetBundle // not downloaded: the CDN refuses it
}
```

### Nested manifests
A downloaded bundle holding exactly two objects, one of them a `TextAsset` named like the bundle without its extension (ignoring case), is a nested manifest: the script of this `TextAsset` is a manifest of the same format, whose records are downloaded in turn.

## Mitake asset bundle
The first `offsetInBytes` bytes of a bundle are a header and are skipped. The rest depends on `cryptoAlgorithm`:
* `None`: stored as is
* `Xor`: not supported by megaturtle-dl, as no current bundle uses it
* `Aes`: an `i32` IV size, the IV, an `i32`, then the encrypted data: its first `0x410` bytes are encrypted with AES-256 in CBC mode with PKCS#7 padding, the rest being stored as is

### AES key derivation
The 256-bit key is made of eight 32-bit little-endian words computed from the `u0`, `u1`, `u2` and `u3` values of the record's `hash`:
| Word | Value |
|---|---|
| 0 | `u1 ^ u0` |
| 1 | `u2 ^ u1` |
| 2 | `u3 ^ u2` |
| 3 | `u3 ^ u0` |
| 4 | `u2 ^ u0` |
| 5 | `u3 ^ u1` |
| 6 | `(u0 << 16) \| (u1 >> 16)` |
| 7 | `(u2 << 16) \| (u3 >> 16)` |
