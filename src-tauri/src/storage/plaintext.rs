//! Plaintext JSONL backend (default).
//!
//! One JSON-encoded [`Event`] per line, appended as sessions finish. Trivial
//! to tail, grep, or import elsewhere — at the cost of being readable by
//! anything that can read the file. The UI warns about this.

use super::Store;
use crate::model::Event;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub struct PlainStore {
    path: PathBuf,
}

impl PlainStore {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Touch the file so the configured path exists immediately.
        OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(PlainStore { path })
    }
}

impl Store for PlainStore {
    fn append(&mut self, e: &Event) -> anyhow::Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(e)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    fn all(&self) -> anyhow::Result<Vec<Event>> {
        let f = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(Vec::new()),
        };
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            // Skip unparseable lines rather than aborting the whole read:
            // a half-written final line (e.g. after a crash) shouldn't make
            // the entire history unreadable.
            if let Ok(e) = serde_json::from_str::<Event>(&line) {
                out.push(e);
            }
        }
        Ok(out)
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        File::create(&self.path)?; // truncate to empty
        Ok(())
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}
