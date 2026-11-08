// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Console output: progress and summaries on stdout, warnings and errors on
//! stderr.

use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering::Relaxed};

static VERBOSITY: AtomicU8 = AtomicU8::new(1); // 0 quiet, 1 normal, 2 verbose
static LABEL: Mutex<&str> = Mutex::new(""); // source being downloaded
static DONE: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static FILES: AtomicUsize = AtomicUsize::new(0);
static FAILS: AtomicUsize = AtomicUsize::new(0);
static PROGRESS: AtomicBool = AtomicBool::new(false); // a progress line is open

/// Sets the verbosity: 0 quiet, 1 normal, 2 verbose.
pub fn set_verbosity(v: u8) {
    VERBOSITY.store(v, Relaxed);
}

/// Returns the source being downloaded.
fn label() -> &'static str {
    *LABEL.lock().unwrap()
}

/// Prints a line in normal and verbose modes.
pub fn say(line: &str) {
    if VERBOSITY.load(Relaxed) > 0 {
        println!("{line}");
    }
}

/// Starts the download of a source of `total` items.
pub fn begin(source: &'static str, total: usize) {
    *LABEL.lock().unwrap() = source;
    DONE.store(0, Relaxed);
    TOTAL.store(total, Relaxed);
    FILES.store(0, Relaxed);
    FAILS.store(0, Relaxed);
}

/// Adds `n` items to the source being downloaded.
pub fn add_total(n: usize) {
    TOTAL.fetch_add(n, Relaxed);
}

/// Counts a file written.
pub fn file_written() {
    FILES.fetch_add(1, Relaxed);
}

/// Counts a bundle that could not be downloaded or read, listed in verbose mode
/// only, as there may be thousands.
pub fn fail(msg: &str) {
    FAILS.fetch_add(1, Relaxed);
    if VERBOSITY.load(Relaxed) > 1 {
        eprintln!("megaturtle-dl: {}: {msg}", label());
    }
}

/// Counts an item done, showing the progress in normal mode. The output is locked
/// while counting, so that threads cannot show their counts out of order.
pub fn tick() {
    if VERBOSITY.load(Relaxed) != 1 {
        DONE.fetch_add(1, Relaxed);
        return;
    }
    let mut out = std::io::stdout().lock();
    let done = DONE.fetch_add(1, Relaxed) + 1;
    let _ = write!(
        out,
        "\r  {}: downloaded {done}/{}",
        label(),
        TOTAL.load(Relaxed)
    );
    let _ = out.flush();
    PROGRESS.store(true, Relaxed);
}

/// Ends the download of a source, printing its summary or its error, and how
/// many bundles failed. Returns whether all went well.
pub fn finish(source: &str, result: anyhow::Result<()>) -> bool {
    if PROGRESS.swap(false, Relaxed) {
        println!(); // ends the progress line
    }
    if let Err(e) = result {
        eprintln!("megaturtle-dl: {source}: {e:#}");
        return false;
    }
    say(&format!(
        "  {source}: {} bundles, {} files written",
        DONE.load(Relaxed),
        FILES.load(Relaxed)
    ));

    let fails = FAILS.load(Relaxed);
    if fails > 0 {
        let _ = std::io::stdout().flush(); // before the message, on another stream
        eprintln!(
            "megaturtle-dl: {source}: {fails} bundle{} could not be downloaded or read{}",
            if fails == 1 { "" } else { "s" },
            if VERBOSITY.load(Relaxed) > 1 {
                ""
            } else {
                " (-v lists them)"
            }
        );
    }
    fails == 0
}
