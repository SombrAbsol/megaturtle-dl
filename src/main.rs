// SPDX-FileCopyrightText: 2023 Razmoth
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Asset bundle downloader for Pokémon HOME, a Rust port of HOMEManager by
//! Razmoth, extended with new features: https://github.com/Razmoth/HOMEManager

mod binary;
mod decryptor;
mod downloader;
mod mitake;
mod parsers;
mod rijndael;
mod state;
mod ui;
mod unity;
mod utils;

use anyhow::{Context, Result, anyhow};
use downloader::{CdnClient, MAX_FAILURES_IN_ROW};
use mitake::{AbDownloadManifest, AbRecord};
use state::Finished;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use ui::PROGRAM_NAME;
use utils::Platform;

/// Concurrent downloads.
const WORKERS: usize = 8;
/// Concurrent HEAD requests of the MDVR detection.
const PROBE_WORKERS: usize = 32;
/// How far above the highest MDVR found the probing goes.
const PROBE_SPAN: i32 = 10000;

/// Options of the command line: short name, long name, whether an argument
/// follows.
const OPTIONS: [(Option<char>, &str, bool); 12] = [
    (Some('l'), "list", false),
    (None, "aba", false),
    (None, "abi", false),
    (None, "mitake-android", false),
    (None, "mitake-ios", false),
    (Some('m'), "mdvr", true),
    (Some('o'), "output", true),
    (Some('v'), "verbose", false),
    (Some('q'), "quiet", false),
    (None, "silent", false),
    (Some('h'), "help", false),
    (None, "version", false),
];

/// What the command line asks for.
enum Command {
    Help,
    Version,
    List,
    Download(Settings),
}

/// Sources to download and where.
#[derive(Default)]
struct Settings {
    list: bool,
    aba: bool,
    abi: bool,
    mitake_android: bool,
    mitake_ios: bool,
    mdvr: Option<i32>,
    output: Option<String>,
}

/// Parses the command line as getopt_long does: short options grouped or with
/// their argument attached, long ones abbreviated or with "=argument". The
/// help and version stop the parsing. Returns the error message on failure.
fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut s = Settings::default();
    let mut operands = false; // past "--"
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" && !operands {
            operands = true;
            continue;
        }
        if operands || !a.starts_with('-') || a == "-" {
            return Err(format!("extra operand '{a}'"));
        }

        // a long option or its only abbreviation, or a group of short ones
        let mut found = Vec::new();
        if let Some(long) = a.strip_prefix("--") {
            let (name, attached) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let candidates: Vec<_> = OPTIONS.iter().filter(|o| o.1.starts_with(name)).collect();
            let option = match OPTIONS.iter().find(|o| o.1 == name) {
                Some(o) => o,
                None if candidates.len() == 1 => candidates[0],
                None if candidates.is_empty() => return Err(format!("unrecognized option '{a}'")),
                None => return Err(format!("option '{a}' is ambiguous")),
            };
            if attached.is_some() && !option.2 {
                return Err(format!("option '--{}' doesn't allow an argument", option.1));
            }
            found.push((option, attached, format!("--{}", option.1)));
        } else {
            let chars: Vec<char> = a.chars().skip(1).collect();
            for (k, &c) in chars.iter().enumerate() {
                let option = OPTIONS
                    .iter()
                    .find(|o| o.0 == Some(c))
                    .ok_or_else(|| format!("invalid option -- '{c}'"))?;
                let rest: String = chars[k + 1..].iter().collect();
                let attached = (option.2 && !rest.is_empty()).then_some(rest);
                let stop = attached.is_some();
                found.push((option, attached, format!("-{c}")));
                if stop {
                    break;
                }
            }
        }

        for (option, attached, shown) in found {
            let arg = match (option.2, attached) {
                (false, _) => None,
                (true, Some(v)) => Some(v),
                (true, None) => {
                    i += 1;
                    let next = args.get(i - 1).cloned();
                    Some(next.ok_or_else(|| format!("option '{shown}' requires an argument"))?)
                }
            };
            match option.1 {
                "list" => s.list = true,
                "aba" => s.aba = true,
                "abi" => s.abi = true,
                "mitake-android" => s.mitake_android = true,
                "mitake-ios" => s.mitake_ios = true,
                "mdvr" => s.mdvr = Some(parse_mdvr(arg.as_deref().unwrap_or(""))?),
                "output" => s.output = arg,
                "verbose" => ui::set_verbosity(2),
                "quiet" | "silent" => ui::set_verbosity(0),
                "help" => return Ok(Command::Help),
                _ => return Ok(Command::Version),
            }
        }
    }

    // listing downloads nothing
    if s.list {
        let source = s.aba || s.abi || s.mitake_android || s.mitake_ios;
        if source || s.mdvr.is_some() || s.output.is_some() {
            return Err("option '--list' conflicts with the download options".into());
        }
        return Ok(Command::List);
    }

    // the sources of the Android release when none is named, or only its packs
    // for an MDVR, which the Mitake asset bundles do not have
    if !s.aba && !s.abi && !s.mitake_android && !s.mitake_ios {
        s.aba = true;
        s.mitake_android = s.mdvr.is_none();
    }
    if !s.aba && !s.abi && s.mdvr.is_some() {
        return Err("option '--mdvr' only applies with --aba or --abi".into());
    }
    Ok(Command::Download(s))
}

/// Parses an MDVR, telling numbers out of range from malformed ones as POSIX
/// does.
fn parse_mdvr(s: &str) -> Result<i32, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid number '{s}'"));
    }
    s.parse()
        .map_err(|_| format!("'{s}' is out of the supported range"))
}

/// Prints the usage, options and their defaults.
fn print_help() {
    println!("Usage: {PROGRAM_NAME} [OPTION]...");
    print!(
        "Download and decrypt the asset bundles of Pokémon HOME.

With no source named, those of the Android release are downloaded: --aba and
--mitake-android, or only --aba with --mdvr. Otherwise, only the sources named
are, in any combination. The bundles are written to the android or ios directory
of OUTPUT, the bundles directory in the current directory by default.

Options:
  -h, --help             show this help and exit
      --version          show the version and exit
  -l, --list             list the master data versions (MDVR) on the CDN and
                         exit
      --aba              download the asset bundle packs for Android (ABA)
      --abi              download the asset bundle packs for iOS (ABI)
      --mitake-android   download the Mitake asset bundles for Android
      --mitake-ios       download the Mitake asset bundles for iOS
  -m, --mdvr=N           master data version of the packs to download (default:
                         the latest one on the CDN)
  -o, --output=OUTPUT    write under the OUTPUT directory
  -v, --verbose          also list the bundles that could not be downloaded or
                         read
  -q, --quiet, --silent  show errors and warnings only
"
    );
}

/// Prints the version, the UTC build date and time followed by the OS built for,
/// the build profile and static linking, if any, then the copyright.
fn print_version() {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let linking = if cfg!(target_feature = "crt-static") {
        "-static"
    } else {
        ""
    };
    println!(
        "{PROGRAM_NAME} {}-{}-UTC-{os}-{profile}{linking}",
        env!("BUILD_DATE"),
        env!("BUILD_TIME")
    );
    print!(
        "Copyright (C) 2026 SombrAbsol
Copyright (C) 2023 Razmoth (HOMEManager code)
Copyright (C) 2016 Radu, 2016-2022 Perfare and 2021-2026 aelurum (AssetStudioMod code)
License MIT (Expat), and those listed in THIRD_PARTY_NOTICES.md
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.
"
    );
}

/// Command-line interface.
fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let settings = match parse_args(argv.get(1..).unwrap_or_default()) {
        Ok(Command::Help) => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            print_version();
            return ExitCode::SUCCESS;
        }
        Ok(Command::List) => return list_mdvrs(),
        Ok(Command::Download(s)) => s,
        Err(e) => {
            eprintln!("{PROGRAM_NAME}: {e}");
            eprintln!("Try '{PROGRAM_NAME} --help' for more information.");
            return ExitCode::FAILURE;
        }
    };
    utils::set_output_dir(PathBuf::from(
        settings.output.as_deref().unwrap_or("bundles"),
    ));

    // a source that fails is reported, then the other is still downloaded
    let mut ok = true;
    let packs = [
        (settings.aba, Platform::Android),
        (settings.abi, Platform::Ios),
    ];
    for (_, platform) in packs.into_iter().filter(|(wanted, _)| *wanted) {
        let source = platform.pack_source();
        ui::begin(source, 0);
        let result = settings
            .mdvr
            .map_or_else(|| detect_mdvr(platform), Ok)
            .and_then(|mdvr| download_abap(mdvr, platform));
        ok &= ui::finish(source, result);
    }
    let mitake = [
        (settings.mitake_android, Platform::Android),
        (settings.mitake_ios, Platform::Ios),
    ];
    for (_, platform) in mitake.into_iter().filter(|(wanted, _)| *wanted) {
        let source = platform.mitake_source();
        ui::begin(source, 0);
        ok &= ui::finish(source, download_mitake(platform));
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Runs `f` on every item with up to `workers` threads, keeping their order.
fn par_map<T: Sync, R: Send>(items: &[T], workers: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new((0..items.len()).map(|_| None).collect::<Vec<Option<R>>>());
    std::thread::scope(|s| {
        for _ in 0..workers.min(items.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .flatten()
        .collect()
}

/// Returns the MDVRs of a platform on the CDN in ascending order, at least one,
/// probing multiples of 100 from `first` up to `PROBE_SPAN` above the highest
/// found, or above `MIN_MDVR` if none is higher. A probe that fails is an error,
/// as the versions would be wrong.
fn available_mdvrs(first: i32, platform: Platform) -> Result<Vec<i32>> {
    let cdn = CdnClient::new(downloader::ABAP_API);
    let mut found: Vec<i32> = Vec::new();
    let mut next = first;
    loop {
        let highest = found
            .last()
            .map_or(utils::MIN_MDVR, |&v| v.max(utils::MIN_MDVR));
        let limit = highest + PROBE_SPAN;
        if next > limit {
            break;
        }

        // the first failure spares the other probes, which would wait as long
        let failed = AtomicBool::new(false);
        let candidates: Vec<i32> = (next..=limit).step_by(100).collect();
        let results = par_map(&candidates, PROBE_WORKERS, |&v| {
            if failed.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let result = cdn.exists(&platform.md_path(v));
            failed.fetch_or(result.is_err(), Ordering::Relaxed);
            result
                .map(|exists| exists.then_some(v))
                .with_context(|| format!("cannot check MDVR {v}"))
        });
        for result in results {
            found.extend(result?);
        }
        next = limit + 100;
    }
    if found.is_empty() {
        return Err(anyhow!("no MDVR found on the CDN from {first}"));
    }
    Ok(found)
}

/// Prints every MDVR on the CDN, one per line. Android and iOS have the same.
fn list_mdvrs() -> ExitCode {
    match available_mdvrs(100, Platform::Android) {
        Ok(found) => {
            found.iter().for_each(|mdvr| println!("{mdvr}"));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{PROGRAM_NAME}: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Returns the highest MDVR of a platform on the CDN, sparing the older ones the
/// listing probes.
fn detect_mdvr(platform: Platform) -> Result<i32> {
    let found = available_mdvrs(utils::MIN_MDVR, platform)?;
    let mdvr = found[found.len() - 1];
    let source = platform.pack_source();
    ui::say(&format!("  {source}: MDVR {mdvr} detected"));
    Ok(mdvr)
}

/// Saves a file listing as downloaded.
fn save_listing(platform: Platform, name: &str, data: &[u8]) -> Result<()> {
    if utils::write_listing(platform, name, data).with_context(|| format!("cannot write {name}"))? {
        ui::file_written();
    }
    Ok(())
}

/// Returns the error ending a download when the CDN seems down.
fn check_up(cdn: &CdnClient) -> Result<()> {
    if cdn.is_down() {
        return Err(anyhow!(
            "stopping after {MAX_FAILURES_IN_ROW} downloads failed in a row"
        ));
    }
    Ok(())
}

/// Downloads and unpacks every asset bundle pack of an MDVR not finished yet.
/// Only an unusable pack list or a CDN that seems down are errors: a pack that
/// fails is counted, and the others go on.
fn download_abap(mdvr: i32, platform: Platform) -> Result<()> {
    let cdn = CdnClient::new(downloader::ABAP_API);
    let data = cdn
        .download_file(&platform.md_path(mdvr))
        .ok_or_else(|| anyhow!("cannot download the pack list of MDVR {mdvr}"))?;
    save_listing(platform, &utils::md_file_name(mdvr), &data)?;
    let items = parsers::read_md(&data, platform).context("cannot read the pack list")?;
    let finished = Finished::open(
        &utils::state_path(platform, &mdvr.to_string()),
        utils::output_dir(),
    )?;
    ui::begin(platform.pack_source(), items.len());
    par_map(&items, WORKERS, |item| {
        if cdn.is_down() {
            return;
        }
        let key = format!("{} {}", item.nm, item.version);
        if !finished.is_complete(&key) {
            let dir = format!("{mdvr}/{}", item.nm);
            let result = cdn
                .download_file(&platform.pack_path(item.version, &item.nm))
                .map(|pack| finished.add(&key, &parsers::parse_abap(&pack, &dir, platform)?));
            if let Some(Err(e)) = result {
                ui::fail(&format!("{}: {e:#}", item.nm));
            }
        }
        ui::tick();
    });
    check_up(&cdn)
}

/// Downloads and unpacks every Mitake asset bundle of a platform not finished
/// yet. Only an unusable root manifest or a CDN that seems down are errors: a
/// bundle that fails is counted, and the others go on.
fn download_mitake(platform: Platform) -> Result<()> {
    let cdn = CdnClient::new(downloader::MITAKE_API);
    let data = cdn
        .download_file(&platform.mitake_manifest_path())
        .ok_or_else(|| anyhow!("cannot download the root manifest"))?;
    save_listing(platform, &platform.mitake_manifest_name(), &data)?;
    let mut pending = AbDownloadManifest::parse(&data)
        .context("cannot read the root manifest")?
        .records
        .ok_or_else(|| anyhow!("the manifest has no records"))?;

    let finished = Finished::open(&utils::state_path(platform, "mitake"), utils::output_dir())?;

    // each wave yields the records of the nested manifests it found
    ui::begin(platform.mitake_source(), pending.len());
    while !pending.is_empty() {
        let found = par_map(&pending, WORKERS, |record| {
            if cdn.is_down() {
                return Vec::new();
            }
            let sub = process_record(&cdn, record, platform, &finished).unwrap_or_else(|e| {
                ui::fail(&format!("{}: {e:#}", record.url(platform)));
                Vec::new()
            });
            ui::tick();
            sub
        });
        check_up(&cdn)?;
        let next: Vec<AbRecord> = found.into_iter().flatten().collect();
        ui::add_total(next.len());
        pending = next;
    }
    Ok(())
}

/// Downloads, unpacks and saves one record, unless finished. The records of a
/// nested manifest are returned, so a manifest is never recorded as finished. A
/// failed download, already counted, yields nothing.
fn process_record(
    cdn: &CdnClient,
    record: &AbRecord,
    platform: Platform,
    finished: &Finished,
) -> Result<Vec<AbRecord>> {
    // the CDN refuses these
    if record.is_streaming_scene_asset_bundle {
        return Ok(Vec::new());
    }
    let key = record.state_key();
    if finished.is_complete(&key) {
        return Ok(Vec::new());
    }
    let Some(bytes) = cdn.download_file(&record.url(platform)) else {
        return Ok(Vec::new());
    };
    let unpacked = record.unpack(&bytes)?;

    let path = record.path(platform)?;
    let stem = utils::file_name_without_extension(&path.to_string_lossy());
    let manifest = unity::manifest(&unpacked, &stem);
    if let Err(e) = &manifest {
        // saved all the same, but if it is a manifest, its bundles are missed
        ui::fail(&format!("cannot read {}: {e:#}", path.display()));
    }
    if utils::write_if_changed(&path, &unpacked)? {
        ui::file_written();
    }
    match manifest {
        Ok(Some(sub)) => Ok(AbDownloadManifest::parse(&sub)?.records.unwrap_or_default()),
        Ok(None) => {
            let size = unpacked.len() as u64;
            finished.add(&key, &[(path, size)])?;
            Ok(Vec::new())
        }
        Err(_) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(&args.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    fn settings(args: &[&str]) -> Settings {
        match parse(args) {
            Ok(Command::Download(s)) => s,
            _ => panic!("no download for {args:?}"),
        }
    }

    /// Returns the sources of a command line: ABA, ABI, Mitake for Android and
    /// for iOS.
    fn sources(args: &[&str]) -> (bool, bool, bool, bool) {
        let s = settings(args);
        (s.aba, s.abi, s.mitake_android, s.mitake_ios)
    }

    fn error(args: &[&str]) -> String {
        match parse(args) {
            Err(e) => e,
            _ => panic!("no error for {args:?}"),
        }
    }

    #[test]
    fn android_sources_by_default() {
        assert_eq!(sources(&[]), (true, false, true, false));
    }

    #[test]
    fn named_sources_replace_the_default() {
        assert_eq!(sources(&["--aba"]), (true, false, false, false));
        assert_eq!(sources(&["--abi"]), (false, true, false, false));
        assert_eq!(sources(&["--mitake-android"]), (false, false, true, false));
        assert_eq!(sources(&["--mitake-ios"]), (false, false, false, true));
        assert_eq!(
            sources(&["--aba", "--abi", "--mitake-android", "--mitake-ios"]),
            (true, true, true, true)
        );
    }

    #[test]
    fn mdvr_alone_means_the_android_packs() {
        assert_eq!(sources(&["-m", "35700"]), (true, false, false, false));
        assert_eq!(sources(&["--mdvr=35700"]), (true, false, false, false));
        assert_eq!(sources(&["-m1", "--abi"]), (false, true, false, false));
        assert_eq!(settings(&["-m", "35700"]).mdvr, Some(35700));
        assert_eq!(settings(&["-m35700"]).mdvr, Some(35700));
        assert_eq!(settings(&["--mdvr", "35700"]).mdvr, Some(35700));
    }

    #[test]
    fn mdvr_needs_packs_and_a_number() {
        let expected = "option '--mdvr' only applies with --aba or --abi";
        assert_eq!(error(&["--mitake-ios", "-m", "1"]), expected);
        assert_eq!(error(&["-m"]), "option '-m' requires an argument");
        assert_eq!(error(&["-m", "abc"]), "invalid number 'abc'");
        assert_eq!(
            error(&["-m", "99999999999"]),
            "'99999999999' is out of the supported range"
        );
    }

    #[test]
    fn output_forms() {
        for args in [
            &["-o", "x"][..],
            &["-ox"],
            &["--output=x"],
            &["--output", "x"],
            &["--out=x"],
        ] {
            assert_eq!(settings(args).output.as_deref(), Some("x"));
        }
    }

    #[test]
    fn list_downloads_nothing() {
        assert!(matches!(parse(&["-l"]), Ok(Command::List)));
        assert!(matches!(parse(&["--list"]), Ok(Command::List)));
        for option in ["--aba", "--abi", "--mitake-ios", "-m1", "-ox"] {
            assert_eq!(
                error(&["-l", option]),
                "option '--list' conflicts with the download options"
            );
        }
    }

    #[test]
    fn help_and_version_stop_the_parsing() {
        assert!(matches!(parse(&["-h", "--nope"]), Ok(Command::Help)));
        assert!(matches!(parse(&["--version", "x"]), Ok(Command::Version)));
    }

    #[test]
    fn long_options_abbreviate_when_unique() {
        assert_eq!(sources(&["--mitake-i"]), (false, false, false, true));
        assert_eq!(error(&["--mitake"]), "option '--mitake' is ambiguous");
        assert_eq!(error(&["--ab"]), "option '--ab' is ambiguous");
    }

    #[test]
    fn invalid_command_lines() {
        assert_eq!(error(&["-a"]), "invalid option -- 'a'");
        assert_eq!(error(&["--nope"]), "unrecognized option '--nope'");
        assert_eq!(
            error(&["--aba=x"]),
            "option '--aba' doesn't allow an argument"
        );
        assert_eq!(error(&["x"]), "extra operand 'x'");
        assert_eq!(error(&["--", "--aba"]), "extra operand '--aba'");
    }
}
