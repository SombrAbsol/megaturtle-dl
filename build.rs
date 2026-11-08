// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Build script embedding the build date and time, shown by --version.

use std::time::{SystemTime, UNIX_EPOCH};

/// Sets BUILD_DATE and BUILD_TIME to the current UTC date and time, as
/// yyyy.mm.dd and hh:mm:ss.
fn main() {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);

    // civil date of a day count since 1970, after Howard Hinnant's algorithm
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);

    let time = secs % 86_400;
    let (hour, minute, second) = (time / 3600, time / 60 % 60, time % 60);

    println!("cargo:rustc-env=BUILD_DATE={year:04}.{month:02}.{day:02}");
    println!("cargo:rustc-env=BUILD_TIME={hour:02}:{minute:02}:{second:02}");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
}
