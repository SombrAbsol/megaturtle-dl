<!--
SPDX-FileCopyrightText: 2026 SombrAbsol

SPDX-License-Identifier: MIT
-->

# megaturtle-dl
<a href="https://github.com/SombrAbsol/megaturtle-dl/actions/workflows/ci.yml"><img src="https://github.com/SombrAbsol/megaturtle-dl/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<a href="https://opensource.org/license/mit"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License: MIT (Expat)"></a>

Asset bundle downloader for *Pokémon HOME*.

The mobile version of *Pokémon HOME* downloads its assets from two content delivery networks: the asset bundle packs (ABA on Android, ABI on iOS), and the Mitake asset bundles. This tool downloads and decrypts these Unity asset bundles, which a Unity asset extraction tool can then open. You can [download the latest build](#download) or [build the program from source](#building).

For more information about how these assets are distributed, see [the documentation](/docs/bundles.md).

## Download
|        | Linux | macOS | Windows |
| ------ | ----- | ----- | ------- |
| Latest | [Download](https://github.com/SombrAbsol/megaturtle-dl/releases/download/latest/megaturtle-dl-linux.zip) | [Download](https://github.com/SombrAbsol/megaturtle-dl/releases/download/latest/megaturtle-dl-macos.zip) | [Download](https://github.com/SombrAbsol/megaturtle-dl/releases/download/latest/megaturtle-dl-windows.zip) |

## Usage
### Running megaturtle-dl
> [!IMPORTANT]
> megaturtle-dl is a command-line program and must be run in a terminal. It needs an Internet connection.

| Source | Android | iOS |
| ------ | ------- | --- |
| Asset bundle packs | `megaturtle-dl --aba` | `megaturtle-dl --abi` |
| Mitake asset bundles | `megaturtle-dl --mitake-android` | `megaturtle-dl --mitake-ios` |

These options can be combined: for instance, `megaturtle-dl --abi --mitake-ios` downloads both sources for iOS. Running `megaturtle-dl` without any of them downloads both sources for Android.

The asset bundle packs are downloaded for the latest master data version (MDVR) found on the server. To download those of another version, run `megaturtle-dl -m N` or `megaturtle-dl --mdvr=N`, where `N` is the version number. This downloads the packs for Android, unless you name `--abi`, alone or with `--aba`.

To list the versions available on the server without downloading anything, run `megaturtle-dl -l` or `megaturtle-dl --list`.

The files are written to a newly created `bundles/` directory in the current directory, `<platform>` being `android` or `ios`:
* the asset bundle packs, as `<platform>/<mdvr>/<pack>/<name>.unity3d`
* the Mitake asset bundles, as `<platform>/<project>/<name>.unity3d`
* the lists of files, as they were downloaded: `<platform>/MD_AssetbundleDLPackVer_<mdvr>.snd` for the asset bundle packs, and `<platform>/MitakeCommon.bin` for the Mitake asset bundles. The other Mitake lists are asset bundles, saved like the others

You can write them elsewhere by running `megaturtle-dl -o <outdir>` or `megaturtle-dl --output=<outdir>` instead, which creates the same directories in `<outdir>`.

When run again, megaturtle-dl skips what it already finished, which also resumes an interrupted download. This is recorded in the `.state/` directory of the output directory, along with the size of each file: a file deleted or of another size is downloaded again, and so is a Mitake asset bundle that changed on the server. Delete this directory to download everything again.

A download that fails for a temporary reason, such as a network or server error, is tried again up to twice. Bundles that still cannot be downloaded, or that cannot be read, do not stop the others, and are counted at the end. A bundle that cannot be downloaded is not written, so an earlier copy is kept. A bundle that cannot be read is still saved, but it may be a manifest listing other bundles, which are then missed. In both cases, megaturtle-dl exits with status 1, and it stops after 20 failed downloads in a row, as the network or the server is then probably down. Run `megaturtle-dl -v` or `megaturtle-dl --verbose` to list them.

Run `megaturtle-dl --help` to list all the options.

### Exporting the assets
megaturtle-dl does not export the content of the `.unity3d` files. To do so, you need a Unity asset extraction tool like [AssetStudioModGUI](https://github.com/aelurum/AssetStudioMod/releases/latest) (Windows only) or [AssetRipper](https://github.com/assetripper/assetripper) (Linux, macOS, Windows).

Launch the tool, then open the `bundles/android/` or `bundles/ios/` directory, or one of their subdirectories to load fewer bundles at once. You can now browse and export the game's assets.

## Building
### Dependencies
* a Rust toolchain, version 1.89 or later (for instance installed with [rustup](https://rustup.rs))
* a C compiler, such as `clang` or `gcc`, used by some dependencies

### Steps
1. If you don't already have them, install the dependencies
2. Clone this repository by running `git clone https://github.com/SombrAbsol/megaturtle-dl`, or [download the ZIP archive](https://github.com/SombrAbsol/megaturtle-dl/archive/refs/heads/main.zip) and extract it
3. Go to the repository directory and build the program by running `cargo build --release`

> [!TIP]
> The program is built in the `target/release` directory. Running `cargo build` instead generates a debug build, which includes debugging symbols that help diagnose issues but runs slower.
>
> You can run `cargo install --path .` to install megaturtle-dl for your user, in the Cargo binary directory. Use `cargo uninstall megaturtle-dl` to remove it.
>
> If you need to rebuild the program, run `cargo clean` or delete the `target` directory.

## Credits
* megaturtle-dl by [SombrAbsol](https://github.com/SombrAbsol)
* Ported from [HOMEManager](https://github.com/Razmoth/HOMEManager), a C# program made by [Razmoth](https://github.com/Razmoth), and extended with new features
* [aelurum](https://github.com/aelurum) for [AssetStudioMod](https://github.com/aelurum/AssetStudioMod), whose Unity asset bundle reader megaturtle-dl adapts

## License
megaturtle-dl is free software. You can redistribute it and/or modify it under the [terms of the Expat License](/LICENSE) as published by the Massachusetts Institute of Technology.

This software is also derived from third-party code and uses third-party libraries, which remain subject to their respective original licenses. See [THIRD_PARTY_NOTICES.md](/THIRD_PARTY_NOTICES.md) for details.
