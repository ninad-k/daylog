//! Storage layer.
//!
//! The capture layer produces [`Event`]s; the storage layer decides how they
//! land on disk. Two interchangeable backends implement the same [`Store`]
//! trait so the rest of the app never cares which is active:
//!
//! * [`plaintext::PlainStore`]  — newline-delimited JSON (JSONL). Human
//!   readable, for testing. This is the default.
//! * [`encrypted::EncryptedStore`] — each event is sealed with AES-256-GCM
//!   under a key derived from the user's password with Argon2id. Only the
//!   Argon2 salt is stored; the password never touches disk.
//!
//! Switching backends prepares a sibling file (`activity.log` vs
//! `activity.enc`). Core migrates history, commits configuration, then removes
//! the source only after the destination is durable.

pub mod encrypted;
pub mod plaintext;

use crate::model::{Event, EventFilter};
use std::path::{Path, PathBuf};

/// Common interface over both on-disk representations.
///
/// `path`/`is_encrypted` round out the backend API (used by tooling and future
/// UI surfaces) even though the current call sites read the path from `Core`.
#[allow(dead_code)]
pub trait Store: Send {
    /// Persist one finished focus session.
    fn append(&mut self, e: &Event) -> anyhow::Result<()>;
    /// Read every stored event (unordered).
    fn all(&self) -> anyhow::Result<Vec<Event>>;
    /// Delete all stored events (keeps the file/header, drops the data).
    fn clear(&mut self) -> anyhow::Result<()>;
    /// Atomically replace the complete history (used by retention and migration).
    fn replace(&mut self, events: &[Event]) -> anyhow::Result<()>;
    /// Absolute path of the active store file.
    fn path(&self) -> PathBuf;
    /// True if this is an encrypted store.
    fn is_encrypted(&self) -> bool {
        false
    }

    /// Query helper shared by both backends: filter, sort newest-first, cap.
    fn query(&self, filter: &EventFilter) -> anyhow::Result<Vec<Event>> {
        let mut v: Vec<Event> = self
            .all()?
            .into_iter()
            .filter(|e| filter.matches(e))
            .collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.start));
        if let Some(limit) = filter.limit {
            v.truncate(limit);
        }
        Ok(v)
    }
}

/// Resolve the active file path for the chosen backend.
pub fn active_path(base_log_path: &Path, encrypted: bool) -> PathBuf {
    if encrypted {
        base_log_path.with_extension("enc")
    } else {
        base_log_path.to_path_buf()
    }
}

/// Replace in the same directory so the rename is atomic and the original survives errors.
pub fn atomic_write(path: &Path, body: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(body)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Appends are durable. Roll back a partial write on error; after a crash,
/// readers tolerate only an incomplete final record, never corrupt middle rows.
pub fn append_line(path: &Path, line: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    let before = file.metadata()?.len();
    let result = file
        .write_all(format!("{line}\n").as_bytes())
        .and_then(|()| file.sync_data());
    if let Err(error) = result {
        file.set_len(before)?;
        file.sync_data()?;
        return Err(error.into());
    }
    Ok(())
}

pub fn read_events(
    path: &Path,
    skip_header: bool,
    parse: impl Fn(&str) -> anyhow::Result<Event>,
) -> anyhow::Result<Vec<Event>> {
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut events = Vec::new();
    let mut line = Vec::new();
    let mut number = 0;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        number += 1;
        if skip_header && number == 1 {
            continue;
        }
        let complete = line.ends_with(b"\n");
        let parsed = std::str::from_utf8(&line)
            .map_err(anyhow::Error::from)
            .and_then(|s| {
                if s.trim().is_empty() {
                    Ok(None)
                } else {
                    parse(s.trim()).map(Some)
                }
            });
        match parsed {
            Ok(Some(event)) => events.push(event),
            Ok(None) => (),
            Err(_) if !complete => break,
            Err(e) => return Err(anyhow::anyhow!("Damaged log record on line {number}: {e}")),
        }
    }
    Ok(deduplicate(events))
}

/// Checkpoints reuse the session id. The latest complete record wins after recovery.
pub fn deduplicate(events: Vec<Event>) -> Vec<Event> {
    let mut unique = std::collections::BTreeMap::new();
    for event in events {
        unique.insert(event.id, event);
    }
    unique.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Event, EventFilter};
    use chrono::{Duration, Utc};

    fn sample() -> Vec<Event> {
        let now = Utc::now();
        vec![
            Event::new(
                now,
                now + Duration::seconds(30),
                "Chrome".into(),
                "Gmail".into(),
            ),
            Event::new(
                now + Duration::seconds(40),
                now + Duration::seconds(100),
                "Code".into(),
                "main.rs".into(),
            ),
        ]
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("daylog-test-{}-{}", std::process::id(), name))
    }

    #[test]
    fn plaintext_roundtrip() {
        let p = tmp("plain.log");
        let _ = std::fs::remove_file(&p);
        let mut s = plaintext::PlainStore::open(p.clone()).unwrap();
        for e in sample() {
            s.append(&e).unwrap();
        }
        assert_eq!(s.all().unwrap().len(), 2);
        s.clear().unwrap();
        assert_eq!(s.all().unwrap().len(), 0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn encrypted_roundtrip_and_wrong_password() {
        let p = tmp("enc.enc");
        let _ = std::fs::remove_file(&p);
        {
            let mut s = encrypted::EncryptedStore::open(p.clone(), "hunter2").unwrap();
            for e in sample() {
                s.append(&e).unwrap();
            }
            assert_eq!(s.all().unwrap().len(), 2);
        }
        // Correct password reopens and decrypts.
        let s2 = encrypted::EncryptedStore::open(p.clone(), "hunter2").unwrap();
        assert_eq!(s2.all().unwrap().len(), 2);
        // Wrong password is rejected (verified against an existing record).
        assert!(encrypted::EncryptedStore::open(p.clone(), "nope").is_err());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn filter_keyword_matches_title() {
        let evs = sample();
        let f = EventFilter {
            keyword: Some("gmail".into()),
            ..Default::default()
        };
        assert_eq!(evs.iter().filter(|e| f.matches(e)).count(), 1);
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::io::Write;

    fn event() -> Event {
        let now = Utc::now();
        Event::new(
            now,
            now + Duration::seconds(10),
            "Code".into(),
            "Document".into(),
        )
    }

    #[test]
    fn empty_and_cleared_encrypted_logs_reject_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.enc");
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        assert!(encrypted::EncryptedStore::open(path.clone(), "wrong").is_err());
        store.append(&event()).unwrap();
        store.clear().unwrap();
        assert!(encrypted::EncryptedStore::open(path, "wrong").is_err());
    }
    #[test]
    fn checkpoints_use_latest_revision() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = plaintext::PlainStore::open(dir.path().join("log")).unwrap();
        let mut e = event();
        store.append(&e).unwrap();
        e.end += Duration::seconds(10);
        e.seconds += 10;
        store.append(&e).unwrap();
        let rows = store.all().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seconds, 20);
    }
    #[test]
    fn incomplete_tail_is_recoverable_but_middle_corruption_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        let mut store = plaintext::PlainStore::open(path.clone()).unwrap();
        store.append(&event()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        write!(f, "{{\"start\":").unwrap();
        assert_eq!(store.all().unwrap().len(), 1);
        writeln!(f).unwrap();
        assert!(store.all().is_err());
    }
    #[test]
    fn encrypted_corruption_is_never_silently_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.enc");
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        store.append(&event()).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "damaged-record").unwrap();
        assert!(store.all().is_err());
    }
    #[test]
    fn legacy_encrypted_headers_upgrade_after_password_verification() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.enc");
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        store.append(&event()).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        let (first, rest) = body.split_once('\n').unwrap();
        let mut header: serde_json::Value = serde_json::from_str(first).unwrap();
        header["version"] = 1.into();
        header.as_object_mut().unwrap().remove("verifier");
        std::fs::write(&path, format!("{header}\n{rest}")).unwrap();
        assert!(encrypted::EncryptedStore::open(path.clone(), "wrong").is_err());
        let upgraded = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        assert_eq!(upgraded.all().unwrap().len(), 1);
        let upgraded_body = std::fs::read_to_string(&path).unwrap();
        let upgraded_header: serde_json::Value =
            serde_json::from_str(upgraded_body.lines().next().unwrap()).unwrap();
        assert_eq!(upgraded_header["version"], 2);
        assert!(upgraded_header["verifier"].is_string());
    }

    #[test]
    fn incomplete_encrypted_tail_recovers_without_losing_prior_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.enc");
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        store.append(&event()).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        write!(file, "half-written").unwrap();
        let recovered = store.all().unwrap();
        assert_eq!(recovered.len(), 1);
        store.replace(&recovered).unwrap();
        assert_eq!(
            encrypted::EncryptedStore::open(path, "correct")
                .unwrap()
                .all()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn failed_atomic_replace_preserves_original() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("directory");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep"), "original").unwrap();
        assert!(atomic_write(&target, b"replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(target.join("keep")).unwrap(),
            "original"
        );
    }
}
