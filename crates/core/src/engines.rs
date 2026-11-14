//! The real [`Oracle`]: Stockfish pool + Maia, with the cache in front of both.

use std::collections::HashMap;
use std::path::Path;

use shakmaty::Chess;

use crate::cache::{hash_key, Cache};
use crate::maia::Maia;
use crate::oracle::{fen4, EngineResult, HumanResult, Oracle};
use crate::stockfish::StockfishPool;
use crate::tree::HISTORY;

pub struct EngineOracle {
    pub stockfish: StockfishPool,
    pub maia: Maia,
    pub cache: Cache,
}

impl EngineOracle {
    /// Start `workers` Stockfish processes, load Maia and open the cache (`None` = memory only).
    /// `syzygy`: a tablebase folder; used only if Stockfish finds tablebases in it, and then
    /// recorded in the engine name (so it reaches the cache keys and the output settings).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        stockfish: &Path,
        workers: usize,
        nodes: u64,
        hash_mb: u32,
        syzygy: Option<&Path>,
        maia_model: &Path,
        cache: Option<&Path>,
    ) -> anyhow::Result<EngineOracle> {
        use anyhow::Context;
        Ok(EngineOracle {
            stockfish: StockfishPool::start(stockfish, workers, nodes, hash_mb, syzygy)?,
            maia: Maia::load(maia_model, 1)?,
            cache: Cache::open(cache).with_context(|| format!("opening cache {:?}", cache))?,
        })
    }

    fn sf_key(&self, pos: &Chess, multipv: usize) -> String {
        let sf = &self.stockfish;
        // "sf2": entries carry cp/mate (DECISIONS 4.5); older "sf" entries are not reused.
        hash_key(&["sf2", &sf.name, &sf.hash_mb.to_string(), &sf.nodes.to_string(), &multipv.to_string(), &fen4(pos)])
    }

    fn maia_key(&self, hist: &[Chess], es: f32, eo: f32) -> String {
        let h = &hist[hist.len().saturating_sub(HISTORY)..];
        let fens: Vec<String> = h.iter().map(fen4).collect();
        hash_key(&["maia", &self.maia.id, &es.to_string(), &eo.to_string(), &fens.join("|")])
    }
}

impl Oracle for EngineOracle {
    fn engine(&self, positions: &[Chess], multipv: usize) -> anyhow::Result<Vec<EngineResult>> {
        let keys: Vec<String> = positions.iter().map(|p| self.sf_key(p, multipv)).collect();
        let mut found: HashMap<String, EngineResult> = HashMap::new();
        let mut todo_keys = Vec::new();
        let mut todo_pos = Vec::new();
        for (k, p) in keys.iter().zip(positions) {
            if found.contains_key(k) || todo_keys.contains(k) {
                continue;
            }
            match self.cache.get::<EngineResult>(k)? {
                Some(r) => {
                    found.insert(k.clone(), r);
                }
                None => {
                    todo_keys.push(k.clone());
                    todo_pos.push(p.clone());
                }
            }
        }
        if !todo_pos.is_empty() {
            let res = self.stockfish.search_many(&todo_pos, multipv)?;
            let items: Vec<(String, EngineResult)> = todo_keys.into_iter().zip(res).collect();
            self.cache.put_many("sf", &items)?;
            found.extend(items);
        }
        Ok(keys.iter().map(|k| found[k].clone()).collect())
    }

    fn human(&self, histories: &[&[Chess]], es: f32, eo: f32) -> anyhow::Result<Vec<HumanResult>> {
        let keys: Vec<String> = histories.iter().map(|h| self.maia_key(h, es, eo)).collect();
        let mut found: HashMap<String, HumanResult> = HashMap::new();
        let mut todo_keys = Vec::new();
        let mut todo_hist: Vec<&[Chess]> = Vec::new();
        for (k, h) in keys.iter().zip(histories) {
            if found.contains_key(k) || todo_keys.contains(k) {
                continue;
            }
            match self.cache.get::<HumanResult>(k)? {
                Some(r) => {
                    found.insert(k.clone(), r);
                }
                None => {
                    todo_keys.push(k.clone());
                    todo_hist.push(h);
                }
            }
        }
        if !todo_hist.is_empty() {
            let res = self.maia.evaluate(&todo_hist, es, eo)?;
            let items: Vec<(String, HumanResult)> = todo_keys.into_iter().zip(res).collect();
            self.cache.put_many("maia", &items)?;
            found.extend(items);
        }
        Ok(keys.iter().map(|k| found[k].clone()).collect())
    }
}
