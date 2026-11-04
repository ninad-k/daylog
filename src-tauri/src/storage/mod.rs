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
//! Switching backends writes to a sibling file (`activity.log` vs
//! `activity.enc`), so flipping the toggle never corrupts existing data.
//! Migrating events between the two formats is intentionally left as a
//! follow-up (see ROADMAP Phase 2).

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
