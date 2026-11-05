//! Plaintext JSONL backend (default).
//!
//! One JSON-encoded [`Event`] per line, appended as sessions finish. Trivial
//! to tail, grep, or import elsewhere — at the cost of being readable by
//! anything that can read the file. The UI warns about this.

use super::Store;
use crate::model::Event;
use std::fs::OpenOptions;
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
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?;
        Ok(PlainStore { path })
    }
}

impl Store for PlainStore {
    fn append(&mut self, e: &Event) -> anyhow::Result<()> {
        super::append_line(&self.path, &serde_json::to_string(e)?)
    }

    fn all(&self) -> anyhow::Result<Vec<Event>> {
        super::read_events(&self.path, false, |line| Ok(serde_json::from_str(line)?))
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        self.replace(&[])
    }

    fn replace(&mut self, events: &[Event]) -> anyhow::Result<()> {
        let mut body = Vec::new();
        for event in events {
            serde_json::to_writer(&mut body, event)?;
            body.push(b'\n');
        }
        super::atomic_write(&self.path, &body)
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }
}
