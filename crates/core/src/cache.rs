//! Transposition cache on disk (SQLite), with an in-memory layer in front.
//! Keys (DECISIONS 0.5):
//!   Stockfish: engine name + hash size + node limit + MultiPV + FEN without move counters
//!   Maia:      model id + both ratings + the last 8 positions
//! Values are JSON of the oracle result types.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};

pub struct Cache {
    conn: Option<Mutex<Connection>>,
    mem: Mutex<HashMap<String, String>>,
    pub hits: Mutex<u64>,
    pub misses: Mutex<u64>,
}

pub fn hash_key(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl Cache {
    /// `path = None` keeps everything in memory for this run only.
    pub fn open(path: Option<&Path>) -> anyhow::Result<Cache> {
        let conn = match path {
            Some(p) => {
                let c = Connection::open(p)?;
                c.execute_batch(
                    "PRAGMA journal_mode=WAL;
                     PRAGMA synchronous=NORMAL;
                     CREATE TABLE IF NOT EXISTS results (key TEXT PRIMARY KEY, kind TEXT NOT NULL, value TEXT NOT NULL);",
                )?;
                Some(Mutex::new(c))
            }
            None => None,
        };
        Ok(Cache { conn, mem: Mutex::new(HashMap::new()), hits: Mutex::new(0), misses: Mutex::new(0) })
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> anyhow::Result<Option<T>> {
        if let Some(v) = self.mem.lock().unwrap().get(key) {
            *self.hits.lock().unwrap() += 1;
            return Ok(Some(serde_json::from_str(v)?));
        }
        if let Some(c) = &self.conn {
            let v: Option<String> = c
                .lock()
                .unwrap()
                .query_row("SELECT value FROM results WHERE key = ?1", params![key], |r| r.get(0))
                .optional()?;
            if let Some(v) = v {
                *self.hits.lock().unwrap() += 1;
                let out = serde_json::from_str(&v)?;
                self.mem.lock().unwrap().insert(key.to_string(), v);
                return Ok(Some(out));
            }
        }
        *self.misses.lock().unwrap() += 1;
        Ok(None)
    }

    pub fn put_many<T: Serialize>(&self, kind: &str, items: &[(String, T)]) -> anyhow::Result<()> {
        let encoded: Vec<(String, String)> = items
            .iter()
            .map(|(k, v)| Ok((k.clone(), serde_json::to_string(v)?)))
            .collect::<anyhow::Result<_>>()?;
        if let Some(c) = &self.conn {
            let mut c = c.lock().unwrap();
            let tx = c.transaction()?;
            {
                let mut st = tx.prepare_cached("INSERT OR REPLACE INTO results (key, kind, value) VALUES (?1, ?2, ?3)")?;
                for (k, v) in &encoded {
                    st.execute(params![k, kind, v])?;
                }
            }
            tx.commit()?;
        }
        let mut mem = self.mem.lock().unwrap();
        for (k, v) in encoded {
            mem.insert(k, v);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!("trickiness-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.sqlite");
        {
            let c = Cache::open(Some(&path)).unwrap();
            c.put_many("t", &[("a".to_string(), vec![1.5f64, 2.0])]).unwrap();
        }
        let c = Cache::open(Some(&path)).unwrap();
        assert_eq!(c.get::<Vec<f64>>("a").unwrap(), Some(vec![1.5, 2.0]));
        assert_eq!(c.get::<Vec<f64>>("b").unwrap(), None);
        drop(c);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
