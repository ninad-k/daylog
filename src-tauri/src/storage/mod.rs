//! Cached plaintext/encrypted stores and transactional format migration.
//! A migration writes the merged history beside the destination, commits config,
//! and only then removes the old file. Errors before commit preserve both histories.

pub mod encrypted;
pub mod plaintext;

use crate::model::{Event, EventFilter};
use anyhow::{anyhow, Context};
use rand::RngCore;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub trait Store: Send {
    fn append(&mut self, event: &Event) -> anyhow::Result<()>;
    /// Cached events, sorted by (start, id) ascending. No file reads or decryption.
    fn events(&self) -> &[Event];
    /// Replace the complete history only after the new file is durably staged.
    fn replace(&mut self, events: &[Event]) -> anyhow::Result<()>;
    fn path(&self) -> PathBuf;
    /// Used only after a staged store has been moved into its committed location.
    fn relocate(&mut self, path: PathBuf);
    #[allow(dead_code)]
    fn is_encrypted(&self) -> bool {
        false
    }
    fn len(&self) -> usize {
        self.events().len()
    }
    #[allow(dead_code)]
    fn all(&self) -> anyhow::Result<Vec<Event>> {
        Ok(self.events().to_vec())
    }
    fn clear(&mut self) -> anyhow::Result<()> {
        self.replace(&[])
    }

    #[allow(dead_code)]
    fn query(&self, filter: &EventFilter) -> anyhow::Result<Vec<Event>> {
        filter.validate()?;
        let matcher = filter.compile();
        Ok(self
            .events()
            .iter()
            .rev()
            .filter(|event| matcher.matches(event))
            .skip(filter.offset.unwrap_or(0))
            .take(filter.limit.unwrap_or(usize::MAX))
            .cloned()
            .collect())
    }
}

pub fn active_path(base_log_path: &Path, encrypted: bool) -> PathBuf {
    if encrypted {
        base_log_path.with_extension("enc")
    } else {
        base_log_path.to_path_buf()
    }
}

pub(super) fn insert_event(events: &mut Vec<Event>, event: Event) {
    let at =
        events.partition_point(|existing| (existing.start, existing.id) <= (event.start, event.id));
    events.insert(at, event);
}

/// Atomic same-directory replacement works for existing files on Windows too.
/// Temporary files are removed on every error path and never reused.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    atomic_write_with(path, |file| {
        file.write_all(bytes)?;
        Ok(())
    })
}

pub(super) fn append_line(path: &Path, line: &str) -> anyhow::Result<()> {
    let mut file = OpenOptions::new().append(true).open(path)?;
    let previous_length = file.metadata()?.len();
    let write = writeln!(file, "{line}").and_then(|()| file.sync_data());
    if let Err(error) = write {
        if let Err(rollback) = file.set_len(previous_length) {
            return Err(anyhow!("could not append activity record: {error}; could not remove partial record: {rollback}"));
        }
        return Err(error.into());
    }
    Ok(())
}

pub(super) fn atomic_write_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let (temporary, mut file) = TemporaryFile::beside(path)?;
    write(&mut file)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary.path, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

struct TemporaryFile {
    path: PathBuf,
    remove_on_drop: bool,
}

impl TemporaryFile {
    fn beside(path: &Path) -> anyhow::Result<(Self, File)> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let name = path
            .file_name()
            .ok_or_else(|| anyhow!("a file path is required"))?
            .to_string_lossy();
        for _ in 0..10 {
            let random = rand::thread_rng().next_u64();
            let temporary_path = parent.join(format!(
                ".{name}.daylog-{}-{random:016x}.tmp",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&temporary_path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            path: temporary_path,
                            remove_on_drop: true,
                        },
                        file,
                    ))
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!("could not reserve a temporary file"))
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub struct MigrationOutcome {
    pub store: Box<dyn Store>,
    /// The new store/config are committed, but an old file could not be deleted.
    pub cleanup_warning: Option<String>,
}

/// Merge both histories without dropping distinct events sharing a timestamp.
/// `persist` must atomically save the configuration for the destination backend.
/// The source remains authoritative if staging, installation, or persistence fails.
pub fn migrate(
    source: &dyn Store,
    destination: PathBuf,
    encrypted: bool,
    password: Option<&str>,
    persist: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<MigrationOutcome> {
    let source_path = source.path();
    if source_path == destination
        || (destination.exists()
            && std::fs::canonicalize(&source_path)? == std::fs::canonicalize(&destination)?)
    {
        return Err(anyhow!("source and destination are the same file; choose a log path without the .enc extension"));
    }
    let password = if encrypted {
        Some(
            password
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("password required to enable encryption"))?,
        )
    } else {
        None
    };
    let destination_existed = destination.exists();
    let (staged, staged_file) = TemporaryFile::beside(&destination)?;
    drop(staged_file);
    if destination_existed {
        std::fs::copy(&destination, &staged.path)?;
    }
    let open = |path: PathBuf| -> anyhow::Result<Box<dyn Store>> {
        if encrypted {
            Ok(Box::new(encrypted::EncryptedStore::open(
                path,
                password.unwrap(),
            )?))
        } else {
            Ok(Box::new(plaintext::PlainStore::open(path)?))
        }
    };
    // Existing destination headers are upgraded only in the staging copy.
    let mut target = open(staged.path.clone())?;
    let mut seen = HashSet::new();
    let mut merged = Vec::with_capacity(source.len() + target.len());
    for event in source.events().iter().chain(target.events()) {
        let identity = (
            event.id,
            event.start,
            event.end,
            event.app.as_str(),
            event.title.as_str(),
            event.seconds,
            event.typed_text.as_deref(),
        );
        if seen.insert(identity) {
            merged.push(event.clone());
        }
    }
    merged.sort_by_key(|event| (event.start, event.id));
    target.replace(&merged)?;
    drop(target);
    // Validate the bytes actually written, including authentication of every record.
    let mut target = open(staged.path.clone())?;
    let mut backup = if destination_existed {
        let (backup, file) = TemporaryFile::beside(&destination)?;
        drop(file);
        std::fs::copy(&destination, &backup.path)?;
        OpenOptions::new()
            .write(true)
            .open(&backup.path)?
            .sync_all()?;
        Some(backup)
    } else {
        None
    };
    std::fs::rename(&staged.path, &destination).context("could not install migrated log")?;
    if let Err(error) = persist() {
        let rollback = if let Some(backup) = &backup {
            std::fs::rename(&backup.path, &destination)
        } else {
            std::fs::remove_file(&destination)
        };
        if let Err(rollback_error) = rollback {
            if let Some(backup) = backup.as_mut() {
                // Preserve the recovery copy if the filesystem prevents rollback.
                backup.remove_on_drop = false;
            }
            let recovery = backup
                .as_ref()
                .map(|backup| backup.path.display().to_string())
                .unwrap_or_else(|| "no previous destination existed".into());
            return Err(anyhow!("configuration was not saved: {error}; could not roll back destination: {rollback_error}; original source is preserved at {}; destination recovery copy: {recovery}", source_path.display()));
        }
        return Err(error.context("configuration was not saved; storage migration rolled back"));
    }
    target.relocate(destination);
    let mut warnings = Vec::new();
    if let Err(error) = std::fs::remove_file(&source_path) {
        warnings.push(format!(
            "migration succeeded, but the old log at {} could not be removed: {error}",
            source_path.display()
        ));
    }
    if let Some(backup) = backup.as_mut() {
        match std::fs::remove_file(&backup.path) {
            Ok(()) => backup.remove_on_drop = false,
            Err(error) => warnings.push(format!(
                "old destination backup at {} could not be removed: {error}",
                backup.path.display()
            )),
        }
    }
    Ok(MigrationOutcome {
        store: target,
        cleanup_warning: if warnings.is_empty() {
            None
        } else {
            Some(warnings.join("; "))
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        std::env::temp_dir().join(format!(
            "daylog-test-{}-{}-{name}",
            std::process::id(),
            rand::thread_rng().next_u64()
        ))
    }

    #[test]
    fn plaintext_roundtrip_and_cached_order() {
        let path = tmp("plain.log");
        let mut store = plaintext::PlainStore::open(path.clone()).unwrap();
        let events = sample();
        store.append(&events[1]).unwrap();
        store.append(&events[0]).unwrap();
        assert_eq!(store.query(&EventFilter::default()).unwrap()[0].app, "Code");
        drop(store);
        let mut store = plaintext::PlainStore::open(path.clone()).unwrap();
        assert_eq!(store.len(), 2);
        store.clear().unwrap();
        assert_eq!(store.len(), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn encrypted_password_checked_after_clear_and_on_empty_store() {
        let path = tmp("encrypted.enc");
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        assert!(encrypted::EncryptedStore::open(path.clone(), "wrong").is_err());
        for event in sample() {
            store.append(&event).unwrap();
        }
        store.clear().unwrap();
        drop(store);
        assert!(encrypted::EncryptedStore::open(path.clone(), "wrong").is_err());
        assert_eq!(
            encrypted::EncryptedStore::open(path.clone(), "correct")
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn migration_merges_destination_and_removes_plaintext_after_commit() {
        let path = tmp("source.log");
        let destination = path.with_extension("enc");
        let events = sample();
        let mut source = plaintext::PlainStore::open(path.clone()).unwrap();
        source.append(&events[0]).unwrap();
        let mut old_target =
            encrypted::EncryptedStore::open(destination.clone(), "correct").unwrap();
        old_target.append(&events[0]).unwrap();
        old_target.append(&events[1]).unwrap();
        drop(old_target);
        let outcome = migrate(&source, destination.clone(), true, Some("correct"), || {
            assert!(path.exists(), "source must remain before config commit");
            Ok(())
        })
        .unwrap();
        assert_eq!(outcome.store.len(), 2);
        assert!(!path.exists());
        assert!(outcome.cleanup_warning.is_none());
        let outcome =
            migrate(outcome.store.as_ref(), path.clone(), false, None, || Ok(())).unwrap();
        assert_eq!(outcome.store.len(), 2);
        assert!(!destination.exists());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_configuration_commit_restores_existing_destination() {
        let path = tmp("rollback.log");
        let destination = path.with_extension("enc");
        let events = sample();
        let mut source = plaintext::PlainStore::open(path.clone()).unwrap();
        source.append(&events[0]).unwrap();
        let mut previous = encrypted::EncryptedStore::open(destination.clone(), "correct").unwrap();
        previous.append(&events[1]).unwrap();
        drop(previous);
        let before = std::fs::read(&destination).unwrap();
        assert!(
            migrate(&source, destination.clone(), true, Some("correct"), || Err(
                anyhow!("simulated config failure")
            ))
            .is_err()
        );
        assert_eq!(std::fs::read(&destination).unwrap(), before);
        assert_eq!(plaintext::PlainStore::open(path.clone()).unwrap().len(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(destination).unwrap();
    }

    #[test]
    fn failed_configuration_commit_removes_new_destination() {
        let path = tmp("new-destination.log");
        let destination = path.with_extension("enc");
        let mut source = plaintext::PlainStore::open(path.clone()).unwrap();
        source.append(&sample()[0]).unwrap();
        assert!(
            migrate(&source, destination.clone(), true, Some("correct"), || {
                Err(anyhow!("simulated config failure"))
            })
            .is_err()
        );
        assert!(path.exists());
        assert!(!destination.exists());
        assert_eq!(plaintext::PlainStore::open(path.clone()).unwrap().len(), 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_encrypted_header_is_upgraded_without_losing_records() {
        let path = tmp("legacy.enc");
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        store.append(&sample()[0]).unwrap();
        drop(store);
        let text = std::fs::read_to_string(&path).unwrap();
        let (header, records) = text.split_once('\n').unwrap();
        let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
        header["version"] = 1.into();
        header.as_object_mut().unwrap().remove("verifier");
        std::fs::write(&path, format!("{header}\n{records}")).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(encrypted::EncryptedStore::open(path.clone(), "wrong").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        assert_eq!(store.len(), 1);
        store.clear().unwrap();
        drop(store);
        assert!(encrypted::EncryptedStore::open(path.clone(), "wrong").is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn encrypted_missing_verifier_and_damaged_record_are_rejected() {
        let path = tmp("damaged.enc");
        let store = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        drop(store);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut header: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        header.as_object_mut().unwrap().remove("verifier");
        std::fs::write(&path, format!("{header}\n")).unwrap();
        assert!(encrypted::EncryptedStore::open(path.clone(), "correct").is_err());
        std::fs::write(&path, format!("{text}invalid-record\n")).unwrap();
        assert!(encrypted::EncryptedStore::open(path.clone(), "correct").is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_password_never_changes_either_history() {
        let path = tmp("password.log");
        let destination = path.with_extension("enc");
        let source = plaintext::PlainStore::open(path.clone()).unwrap();
        encrypted::EncryptedStore::open(destination.clone(), "correct").unwrap();
        let before = std::fs::read(&destination).unwrap();
        assert!(migrate(&source, destination.clone(), true, Some("wrong"), || Ok(())).is_err());
        assert!(path.exists());
        assert_eq!(std::fs::read(&destination).unwrap(), before);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(destination).unwrap();
    }

    #[test]
    fn migration_rejects_same_path_and_damaged_destination() {
        let path = tmp("same.enc");
        let source = plaintext::PlainStore::open(path.clone()).unwrap();
        assert!(migrate(&source, path.clone(), true, Some("correct"), || Ok(())).is_err());
        let alias = path
            .parent()
            .unwrap()
            .join(".")
            .join(path.file_name().unwrap());
        assert!(migrate(&source, alias, true, Some("correct"), || Ok(())).is_err());
        let destination = path.with_extension("damaged");
        std::fs::write(&destination, "not JSON\n").unwrap();
        assert!(migrate(&source, destination.clone(), false, None, || Ok(())).is_err());
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "not JSON\n");
        assert!(path.exists());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(destination).unwrap();
    }

    #[test]
    fn migration_keeps_distinct_sessions_with_the_same_id() {
        let path = tmp("collision.log");
        let destination = path.with_extension("other.log");
        let mut first = sample().remove(0);
        let mut second = first.clone();
        first.title = "First document".into();
        second.title = "Second document".into();
        let mut source = plaintext::PlainStore::open(path.clone()).unwrap();
        source.append(&first).unwrap();
        let mut previous = plaintext::PlainStore::open(destination.clone()).unwrap();
        previous.append(&second).unwrap();
        drop(previous);
        let outcome = migrate(&source, destination.clone(), false, None, || Ok(())).unwrap();
        assert_eq!(outcome.store.len(), 2);
        assert_ne!(
            outcome.store.events()[0].title,
            outcome.store.events()[1].title
        );
        assert!(!path.exists());
        std::fs::remove_file(destination).unwrap();
    }

    #[test]
    fn typed_text_stays_encrypted_and_migration_identity_includes_text() {
        let path = tmp("typed-source.enc");
        let destination = tmp("typed-destination.enc");
        let plain = tmp("typed-plaintext.log");
        let mut event = sample().remove(0);
        event.typed_text = Some("first recorded document content".into());
        let mut source = encrypted::EncryptedStore::open(path.clone(), "correct").unwrap();
        source.append(&event).unwrap();
        let mut different = event.clone();
        different.typed_text = Some("different recorded document content".into());
        let mut previous = encrypted::EncryptedStore::open(destination.clone(), "correct").unwrap();
        previous.append(&different).unwrap();
        drop(previous);
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("first recorded document content"));
        assert!(migrate(&source, plain.clone(), false, None, || Ok(())).is_err());
        assert!(!plain.exists());
        assert!(path.exists());
        let outcome = migrate(&source, destination.clone(), true, Some("correct"), || {
            Ok(())
        })
        .unwrap();
        assert_eq!(outcome.store.len(), 2);
        assert_eq!(
            outcome
                .store
                .query(&EventFilter {
                    keyword: Some("\"first recorded\"".into()),
                    ..EventFilter::default()
                })
                .unwrap()
                .len(),
            1
        );
        let mut plaintext = plaintext::PlainStore::open(plain.clone()).unwrap();
        assert!(plaintext.append(&event).is_err());
        assert!(plaintext.replace(&[event]).is_err());
        assert_eq!(std::fs::metadata(&plain).unwrap().len(), 0);
        std::fs::remove_file(plain).unwrap();
        std::fs::remove_file(destination).unwrap();
    }

    #[test]
    fn paged_queries_use_cached_events_and_order_tied_timestamps_by_id() {
        let path = tmp("cached.log");
        let mut events = sample();
        events[1].start = events[0].start;
        events[1].id = events[0].id + 1;
        let mut store = plaintext::PlainStore::open(path.clone()).unwrap();
        store.append(&events[1]).unwrap();
        store.append(&events[0]).unwrap();
        // No disk read is required for queries after the initial open.
        std::fs::remove_file(path).unwrap();
        let rows = store
            .query(&EventFilter {
                offset: Some(1),
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, events[0].id);
        assert!(store.append(&events[0]).is_err());
        assert_eq!(store.len(), 2, "failed writes must not enter the cache");
    }

    #[test]
    fn atomic_write_failure_preserves_previous_file() {
        let path = tmp("atomic.log");
        std::fs::write(&path, b"original").unwrap();
        assert!(atomic_write_with(&path, |file| {
            file.write_all(b"partial")?;
            Err(anyhow!("simulated write failure"))
        })
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        atomic_write(&path, b"replacement").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        std::fs::remove_file(path).unwrap();
    }
}
