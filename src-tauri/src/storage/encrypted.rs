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
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use zeroize::Zeroizing;

const MAGIC: &str = "daylog-enc";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Header {
    magic: String,
    version: u32,
    salt: String, // base64
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
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
    /// If the file already holds records, the password is verified by trying to
    /// decrypt the first record; a wrong password fails fast here instead of
    /// silently returning garbage.
    pub fn open(path: PathBuf, password: &str) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (header, header_line, existing_record): (Header, String, Option<String>) =
            match File::open(&path) {
                Ok(f) => {
                    let mut lines = BufReader::new(f).lines();
                    match lines.next() {
                        Some(first) => {
                            let hl = first?;
                            let header: Header = serde_json::from_str(&hl)
                                .context("log header is not a valid daylog encrypted header")?;
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
                Err(_) => {
                    let h = Header::fresh();
                    let hl = serde_json::to_string(&h)?;
                    (h, hl, None)
                }
            };

        let key = derive_key(password, &header)?;

        // Ensure the file exists and begins with the header line.
        if !path.exists() || std::fs::metadata(&path)?.len() == 0 {
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)?;
            writeln!(f, "{header_line}")?;
        }

        let store = EncryptedStore {
            path,
            key,
            header_line,
        };

        // Verify the password against any existing record.
        if let Some(rec) = existing_record {
            store
                .decrypt_line(&rec)
                .map_err(|_| anyhow!("wrong password"))?;
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
    let salt = B64.decode(&header.salt)?;
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
        let line = self.encrypt_event(e)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    fn all(&self) -> anyhow::Result<Vec<Event>> {
        let f = match File::open(&self.path) {
            Ok(f) => f,
            Err(_) => return Ok(Vec::new()),
        };
        let mut out = Vec::new();
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let line = line?;
            if i == 0 || line.trim().is_empty() {
                continue; // header / blank
            }
            if let Ok(e) = self.decrypt_line(&line) {
                out.push(e);
            }
        }
        Ok(out)
    }

    fn clear(&mut self) -> anyhow::Result<()> {
        // Rewrite with just the header so the SAME key keeps working.
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        writeln!(f, "{}", self.header_line)?;
        Ok(())
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    fn is_encrypted(&self) -> bool {
        true
    }
}
