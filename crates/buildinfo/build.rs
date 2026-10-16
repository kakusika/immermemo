//! Stamps a release build: the commit it was built from, a fingerprint of the
//! uncommitted changes on top of it, and when. See the crate doc for why this is
//! a crate of its own.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Only a release build (what goes onto a phone) carries a time. Stamping
    // a debug build would make every `cargo run` recompile the app, with
    // nothing gained: it runs where it was just built.
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    if release {
        // A path that never exists: Cargo treats a missing file as changed, so
        // this script runs on every build and the time is the build's own.
        println!("cargo:rerun-if-changed=always-rerun-so-the-stamp-is-fresh");
    }

    let commit = git(&["rev-parse", "--short=8", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    // Under `jj` the Git `HEAD` is the *parent* of the change being edited, so
    // it stays the same however many edits are made on top of it. The edits
    // themselves are what tell two builds apart: hashed, they make the stamp
    // change with the source and only with it.
    let fingerprint = if release { tree_fingerprint() } else { None };

    println!("cargo:rustc-env=IMMERMEMO_BUILD_COMMIT={commit}");
    println!(
        "cargo:rustc-env=IMMERMEMO_BUILD_CHANGES={}",
        fingerprint.unwrap_or_default()
    );
    let time = if release { utc_now() } else { String::new() };
    println!("cargo:rustc-env=IMMERMEMO_BUILD_TIME={time}");
}

/// What goes into a build: the sources, not the notes and docs beside them.
const BUILD_INPUTS: [&str; 5] = ["app", "crates", "patches", "Cargo.toml", "Cargo.lock"];

/// An 8-digit hex fingerprint of everything in [`BUILD_INPUTS`] that differs
/// from `HEAD` -- tracked changes and files not yet added -- or an empty string
/// when there are none. `None` when Git cannot be asked.
fn tree_fingerprint() -> Option<String> {
    let root = git(&["rev-parse", "--show-toplevel"])?;
    let in_root = |args: &[&str]| -> Option<Vec<u8>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .ok()?;
        output.status.success().then_some(output.stdout)
    };
    let mut diff_args = vec!["diff", "HEAD", "--binary", "--"];
    diff_args.extend(BUILD_INPUTS);
    let diff = in_root(&diff_args)?;
    let mut new_args = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
    new_args.extend(BUILD_INPUTS);
    let new_files = in_root(&new_args)?;

    let mut hash = Fnv::new();
    hash.write(&diff);
    for path in new_files.split(|&byte| byte == 0).filter(|p| !p.is_empty()) {
        hash.write(path);
        let relative = String::from_utf8_lossy(path).into_owned();
        hash.write(&std::fs::read(std::path::Path::new(&root).join(relative)).unwrap_or_default());
    }
    if diff.is_empty() && new_files.is_empty() {
        return Some(String::new());
    }
    Some(format!("{:08x}", hash.finish() as u32))
}

/// 64-bit FNV-1a: a fingerprint, not a defence. Written out so the stamp does not
/// depend on a hasher whose output a Rust release may change.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// `YYYY-MM-DD HH:MM` in UTC, from the system clock.
fn utc_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60
    )
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
