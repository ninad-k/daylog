//! Encrypted backend: AES-256-GCM per record, Argon2id-derived key.
//!
//! File layout (newline-delimited):
//!   line 0  : JSON header  { magic, version, salt(b64), m_cost, t_cost, p_cost }
//!   line 1..: one record per finished session, b64( nonce[12] || ciphertext+tag )
//!             where the plaintext is the JSON encoding of an `Event`.
//!
//! The password is turned into a 32-byte key with Argon2id using the salt from
//! the header. The salt is the only key-material written to disk — the password
//! itself is never persisted, and the derived key is held in a `Zeroizing`
//! buffer so it is wiped from memory on drop.

use super::Store;
use crate::model::Event;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use zeroize::Zeroizing;

const MAGIC: &str = "daylog-enc";
const VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
struct Header {
    magic: String,
    version: u32,
    salt: String, // base64
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
        Header {
            magic: MAGIC.to_string(),
            version: VERSION,
            salt: B64.encode(salt),
            // Modest Argon2id cost: ~19 MiB, 2 passes — fast to unlock, still
            // expensive to brute-force. Tune up for the commercial build.
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
}

impl EncryptedStore {
    /// Open (or create) an encrypted store at `path`, unlocking with `password`.
    ///
    /// Authenticate the password with the header verifier. Legacy v1 logs use
    /// their first record for verification before upgrading the header.
    pub fn open(path: PathBuf, password: &str) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (mut header, header_line, existing_record): (Header, String, Option<String>) =
            match File::open(&path) {
                Ok(f) => {
                    let mut lines = BufReader::new(f).lines();
                    match lines.next() {
                        Some(first) => {
                            let hl = first?;
                            let header: Header = serde_json::from_str(&hl)
                                .context("log header is not a valid daylog encrypted header")?;
                            if header.version == 0 || header.version > VERSION {
                                return Err(anyhow!("unsupported encrypted log version"));
                            }
                            if header.version >= 2 && header.verifier.is_none() {
                                return Err(anyhow!("missing encrypted password verifier"));
                            }
                            if header.magic != MAGIC {
                                return Err(anyhow!("not a daylog encrypted log"));
                            }
                            // Grab the first non-empty record for verification.
                            let mut rec = None;
                            for l in lines {
                                let l = l?;
                                if !l.trim().is_empty() {
                                    rec = Some(l);
                                    break;
                                }
                            }
                            (header, hl, rec)
                        }
                        // Empty file → initialise a fresh header.
                        None => {
                            let h = Header::fresh();
                            let hl = serde_json::to_string(&h)?;
                            (h, hl, None)
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let h = Header::fresh();
                    let hl = serde_json::to_string(&h)?;
                    (h, hl, None)
                }
                Err(e) => return Err(e.into()),
            };

        let key = derive_key(password, &header)?;

        let mut store = EncryptedStore {
            path,
            key,
            header_line,
        };
        if let Some(verifier) = &header.verifier {
            let event = store
                .decrypt_line(verifier)
                .context("wrong password or damaged log header")?;
            if event.app != "daylog-password-verifier" {
                return Err(anyhow!("invalid password verifier"));
            }
        } else if let Some(rec) = &existing_record {
            store
                .decrypt_line(rec)
                .context("wrong password or damaged first record")?;
        }
        // Upgrade legacy headers after successful verification. A verifier also
        // authenticates empty/cleared logs, which have no activity records to test.
        if header.verifier.is_none() {
            let events = if store.path.exists() {
                store.all()?
            } else {
                Vec::new()
            };
            let now = chrono::Utc::now();
            header.verifier = Some(store.encrypt_event(&Event::new(
                now,
                now,
                "daylog-password-verifier".into(),
                String::new(),
            ))?);
            header.version = VERSION;
            store.header_line = serde_json::to_string(&header)?;
            store.replace(&events)?;
        }

        Ok(store)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*self.key))
    }

    fn encrypt_event(&self, e: &Event) -> anyhow::Result<String> {
        let plaintext = serde_json::to_vec(e)?;
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
            .map_err(|e| anyhow!("encrypt failed: {e}"))?;
        let mut blob = Vec::with_capacity(12 + ct.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ct);
        Ok(B64.encode(blob))
    }

    fn decrypt_line(&self, line: &str) -> anyhow::Result<Event> {
        let blob = B64.decode(line.trim())?;
        if blob.len() < 12 {
            return Err(anyhow!("record too short"));
        }
        let (nonce, ct) = blob.split_at(12);
        let pt = self
            .cipher()
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|e| anyhow!("decrypt failed: {e}"))?;
        Ok(serde_json::from_slice(&pt)?)
    }
}

fn derive_key(password: &str, header: &Header) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    anyhow::ensure!(
        (8192..=262144).contains(&header.m_cost)
            && (1..=10).contains(&header.t_cost)
            && (1..=8).contains(&header.p_cost),
        "unsafe or unsupported Argon2 parameters"
    );
    let salt = B64.decode(&header.salt)?;
    anyhow::ensure!(salt.len() == 16, "invalid salt length");
    let params = Params::new(header.m_cost, header.t_cost, header.p_cost, Some(32))
        .map_err(|e| anyhow!("bad argon2 params: {e}"))?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    a2.hash_password_into(password.as_bytes(), &salt, &mut *key)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

impl Store for EncryptedStore {
    fn append(&mut self, e: &Event) -> anyhow::Result<()> {
        super::append_line(&self.path, &self.encrypt_event(e)?)
    }

    fn all(&self) -> anyhow::Result<Vec<Event>> {
        super::read_events(&self.path, true, |line| self.decrypt_line(line))
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        self.replace(&[])
    }

    fn replace(&mut self, events: &[Event]) -> anyhow::Result<()> {
        let mut body = self.header_line.clone();
        body.push('\n');
        for event in events {
            body.push_str(&self.encrypt_event(event)?);
            body.push('\n');
        }
        super::atomic_write(&self.path, body.as_bytes())
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    fn is_encrypted(&self) -> bool {
        true
    }
}
