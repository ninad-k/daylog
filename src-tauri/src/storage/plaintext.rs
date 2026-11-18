//! Plaintext JSONL storage, loaded once and kept in sync with successful writes.

use super::{append_line, atomic_write_with, insert_event, Store};
use crate::model::Event;
use anyhow::Context;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub struct PlainStore {
    path: PathBuf,
    events: Vec<Event>,
}

impl PlainStore {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(&path)?;
        let mut events = Vec::new();
        for (index, line) in BufReader::new(File::open(&path)?).lines().enumerate() {
            let line = line?;
            if !line.trim().is_empty() {
                // Never silently discard damaged history during a migration or purge.
                events.push(serde_json::from_str(&line).with_context(|| {
                    format!(
                        "invalid activity record on line {} in {}",
                        index + 1,
                        path.display()
                    )
                })?);
            }
        }
        events.sort_by_key(|event: &Event| (event.start, event.id));
        Ok(Self { path, events })
    }
}

impl Store for PlainStore {
    fn append(&mut self, event: &Event) -> anyhow::Result<()> {
        anyhow::ensure!(event.typed_text.as_ref().is_none_or(|text| text.is_empty()), "recorded text requires encrypted storage; delete recorded text before disabling encryption");
        let line = serde_json::to_string(event)?;
        append_line(&self.path, &line)?;
        insert_event(&mut self.events, event.clone());
        Ok(())
    }

    fn events(&self) -> &[Event] {
        &self.events
    }

    fn replace(&mut self, events: &[Event]) -> anyhow::Result<()> {
        anyhow::ensure!(!events.iter().any(|event| event.typed_text.as_ref().is_some_and(|text| !text.is_empty())), "recorded text requires encrypted storage; delete recorded text before disabling encryption");
        atomic_write_with(&self.path, |file| {
            for event in events {
                serde_json::to_writer(&mut *file, event)?;
                writeln!(file)?;
            }
            Ok(())
        })?;
        self.events = events.to_vec();
        self.events.sort_by_key(|event| (event.start, event.id));
        Ok(())
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    fn relocate(&mut self, path: PathBuf) {
        self.path = path;
    }
}
