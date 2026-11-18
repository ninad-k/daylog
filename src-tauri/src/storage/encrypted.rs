//! AES-256-GCM records, with an Argon2id-derived key and authenticated header.
//! Decrypted events are cached only in memory for the lifetime of an unlocked store.

use super::{append_line, atomic_write_with, insert_event, Store};
use crate::model::Event;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use zeroize::Zeroizing;

const MAGIC: &str = "daylog-enc";
const VERSION: u32 = 2;
const VERIFICATION: &[u8] = b"daylog password verification v2";

#[derive(Serialize, Deserialize)]
struct Header {
    magic: String,
    version: u32,
    salt: String,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    #[serde(default)]
    verifier: Option<String>,
}

impl Header {
    fn fresh() -> Self {
        let mut salt = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        Self {
            magic: MAGIC.into(),
            version: VERSION,
            salt: B64.encode(salt),
            m_cost: 19_456,
            t_cost: 2,
            p_cost: 1,
            verifier: None,
        }
    }
}

pub struct EncryptedStore {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    header_line: String,
    events: Vec<Event>,
}

impl EncryptedStore {
    /// Version-one logs are read and upgraded without changing their key.
    /// The verifier also rejects a wrong password after every event is cleared.
    pub fn open(path: PathBuf, password: &str) -> anyhow::Result<Self> {
        if password.is_empty() {
            return Err(anyhow!("password cannot be empty"));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let existing = match File::open(&path) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        let mut fresh = false;
        let mut header = if let Some(file) = existing {
            let mut lines = BufReader::new(file).lines();
            match lines.next() {
                Some(line) => {
                    let header: Header = serde_json::from_str(&line?)
                        .context("log header is not a valid daylog encrypted header")?;
                    if header.magic != MAGIC || !matches!(header.version, 1 | VERSION) {
                        return Err(anyhow!("unsupported daylog encrypted log format"));
                    }
                    for line in lines {
                        let line = line?;
                        if !line.trim().is_empty() {
                            records.push(line);
                        }
                    }
                    header
                }
                None => {
                    fresh = true;
                    Header::fresh()
                }
            }
        } else {
            fresh = true;
            Header::fresh()
        };
        let key = derive_key(password, &header)?;
        let mut store = Self {
            path,
            key,
            header_line: String::new(),
            events: Vec::new(),
        };
        if let Some(verifier) = &header.verifier {
            let actual = store
                .decrypt(verifier)
                .map_err(|_| anyhow!("wrong password or damaged encrypted header"))?;
            if actual.as_slice() != VERIFICATION {
                return Err(anyhow!("wrong password or damaged encrypted header"));
            }
        } else if header.version == VERSION && !fresh {
            return Err(anyhow!("encrypted header is missing its password verifier"));
        }
        for (index, record) in records.iter().enumerate() {
            let plaintext = store.decrypt(record).with_context(|| {
                format!("wrong password or damaged encrypted record {}", index + 1)
            })?;
            store.events.push(
                serde_json::from_slice(&plaintext)
                    .with_context(|| format!("invalid encrypted activity record {}", index + 1))?,
            );
        }
        store.events.sort_by_key(|event| (event.start, event.id));
        let upgrade = header.verifier.is_none();
        if upgrade {
            header.version = VERSION;
            header.verifier = Some(store.encrypt(VERIFICATION)?);
        }
        store.header_line = serde_json::to_string(&header)?;
        if upgrade {
            // Upgrade/create only after the entire existing log was authenticated.
            let events = store.events.clone();
            store.replace(&events)?;
        }
        Ok(store)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*self.key))
    }

    fn encrypt(&self, plaintext: &[u8]) -> anyhow::Result<String> {
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| anyhow!("could not encrypt activity record"))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ciphertext);
        Ok(B64.encode(blob))
    }

    fn decrypt(&self, line: &str) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        let blob = B64.decode(line.trim())?;
        if blob.len() < 28 {
            return Err(anyhow!("encrypted record too short"));
        }
        let (nonce, ciphertext) = blob.split_at(12);
        self.cipher()
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map(Zeroizing::new)
            .map_err(|_| anyhow!("encrypted record authentication failed"))
    }
}

fn derive_key(password: &str, header: &Header) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let salt = B64.decode(&header.salt)?;
    // A damaged file must not request unbounded memory or CPU while unlocking.
    if !(8..=262_144).contains(&header.m_cost)
        || !(1..=10).contains(&header.t_cost)
        || !(1..=8).contains(&header.p_cost)
        || salt.len() != 16
    {
        return Err(anyhow!(
            "invalid or excessive Argon2 parameters in log header"
        ));
    }
    let params = Params::new(header.m_cost, header.t_cost, header.p_cost, Some(32))
        .map_err(|error| anyhow!("bad Argon2 parameters: {error}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), &salt, &mut *key)
        .map_err(|error| anyhow!("key derivation failed: {error}"))?;
    Ok(key)
}

impl Store for EncryptedStore {
    fn append(&mut self, event: &Event) -> anyhow::Result<()> {
        let plaintext = Zeroizing::new(serde_json::to_vec(event)?);
        let line = self.encrypt(&plaintext)?;
        append_line(&self.path, &line)?;
        insert_event(&mut self.events, event.clone());
        Ok(())
    }

    fn events(&self) -> &[Event] {
        &self.events
    }

    fn replace(&mut self, events: &[Event]) -> anyhow::Result<()> {
        atomic_write_with(&self.path, |file| {
            writeln!(file, "{}", self.header_line)?;
            for event in events {
                let plaintext = Zeroizing::new(serde_json::to_vec(event)?);
                writeln!(file, "{}", self.encrypt(&plaintext)?)?;
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

    fn is_encrypted(&self) -> bool {
        true
    }
}
