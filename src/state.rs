// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Record of the downloads finished, to skip them when run again or to resume.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// File written and its size.
pub type WrittenFile = (PathBuf, u64);

/// Lines of a file, one per download finished: its key, the number of files it
/// wrote, then their paths and sizes, all separated by tabs. A line that does not
/// match this, such as one cut by an interruption, only means that a download is
/// done again.
pub struct Finished {
    /// Directory the paths recorded are relative to.
    root: PathBuf,
    known: HashMap<String, Vec<(String, u64)>>,
    file: Mutex<File>,
}

/// Reads a line of the record. Returns its key and its files.
fn parse_line(line: &str) -> Option<(String, Vec<(String, u64)>)> {
    let mut fields = line.split('\t');
    let key = fields.next()?.to_string();
    let count: usize = fields.next()?.parse().ok()?;
    let fields: Vec<&str> = fields.collect();
    if fields.len() != count * 2 {
        return None;
    }
    let files = fields
        .chunks(2)
        .map(|f| Some((f[0].to_string(), f[1].parse().ok()?)))
        .collect::<Option<_>>()?;
    Some((key, files))
}

impl Finished {
    /// Opens the record at `path`, created if missing, of files under `root`.
    pub fn open(path: &Path, root: &Path) -> Result<Self> {
        let name = path.display();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("cannot create {name}"))?;
        }
        let file = File::options()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("cannot open {name}"))?;
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Ok(Self {
            root: root.to_path_buf(),
            known: text.lines().filter_map(parse_line).collect(),
            file: Mutex::new(file),
        })
    }

    /// Checks whether an earlier run finished `key` and all its files are still
    /// there, with the same size.
    pub fn is_complete(&self, key: &str) -> bool {
        self.known.get(key).is_some_and(|files| {
            files.iter().all(|(path, size)| {
                std::fs::metadata(self.root.join(path)).is_ok_and(|m| m.len() == *size)
            })
        })
    }

    /// Records that `key` is finished, having written `files`.
    pub fn add(&self, key: &str, files: &[WrittenFile]) -> Result<()> {
        let mut line = format!("{key}\t{}", files.len());
        for (path, size) in files {
            let relative = path.strip_prefix(&self.root).unwrap_or(path);
            line.push_str(&format!("\t{}\t{size}", relative.display()));
        }
        line.push('\n');
        // in one write, so that threads do not mix their lines
        self.file
            .lock()
            .unwrap()
            .write_all(line.as_bytes())
            .context("cannot update the record")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns an empty directory for a test.
    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("megaturtle_dl_state_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remembers_across_runs() {
        let dir = test_dir("runs");
        let path = dir.join("sub").join("record");
        let a = dir.join("a");
        std::fs::write(&a, b"123").unwrap();

        let first = Finished::open(&path, &dir).unwrap();
        assert!(!first.is_complete("x 1"));
        first.add("x 1", &[(a, 3)]).unwrap();
        first.add("empty", &[]).unwrap();

        // this run only knows what earlier ones finished
        assert!(!first.is_complete("x 1"));
        let second = Finished::open(&path, &dir).unwrap();
        assert!(second.is_complete("x 1"));
        assert!(second.is_complete("empty"));
        assert!(!second.is_complete("x 2"));
    }

    #[test]
    fn notices_files_deleted_or_of_another_size() {
        let dir = test_dir("files");
        let path = dir.join("record");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, b"123").unwrap();
        std::fs::write(&b, b"45").unwrap();
        let record = Finished::open(&path, &dir).unwrap();
        record.add("x", &[(a.clone(), 3), (b.clone(), 2)]).unwrap();
        let is_complete = || Finished::open(&path, &dir).unwrap().is_complete("x");
        assert!(is_complete());

        std::fs::write(&b, b"4").unwrap();
        assert!(!is_complete());
        std::fs::write(&b, b"45").unwrap();
        assert!(is_complete());
        std::fs::remove_file(&a).unwrap();
        assert!(!is_complete());
    }

    #[test]
    fn stays_valid_when_the_directory_moves() {
        let dir = test_dir("moved");
        let path = dir.join("record");
        std::fs::create_dir(dir.join("new")).unwrap();
        std::fs::write(dir.join("new").join("a"), b"123").unwrap();

        let old_root = dir.join("old");
        let record = Finished::open(&path, &old_root).unwrap();
        record.add("x", &[(old_root.join("a"), 3)]).unwrap();
        let moved = Finished::open(&path, &dir.join("new")).unwrap();
        assert!(moved.is_complete("x"));
    }

    #[test]
    fn ignores_malformed_lines() {
        assert_eq!(parse_line("old format"), None);
        assert_eq!(parse_line("x\t2\ta\t3"), None); // cut
        assert_eq!(parse_line("x\t1\ta\tb"), None);
        assert_eq!(parse_line("x\ty"), None);
        assert_eq!(
            parse_line("x\t1\ta\t3"),
            Some(("x".into(), vec![("a".into(), 3)]))
        );
        assert_eq!(parse_line("x\t0"), Some(("x".into(), Vec::new())));
    }
}
