//! Stockfish over UCI: a pool of single-threaded processes (DECISIONS 0.12).
//! Each search starts from a cleared hash (`ucinewgame`), so a result depends only on the
//! position, node limit and MultiPV (DECISIONS 0.10).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context};
use shakmaty::Chess;

use crate::oracle::{full_fen, EngineLine, EngineResult, Wdl};

pub struct Stockfish {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    multipv: usize,
    pub name: String,
}

#[derive(Default)]
struct Slot {
    exact: Option<EngineLine>,
    bound: Option<EngineLine>,
}

/// Parse one `info` line. Returns (multipv index, line, is_bound) for lines carrying a PV
/// and WDL.
pub fn parse_info(line: &str) -> Option<(usize, EngineLine, bool)> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    if tok.first() != Some(&"info") {
        return None;
    }
    let get = |key: &str| tok.iter().position(|t| *t == key);
    let pv = get("pv")?;
    let w = get("wdl")?;
    let mpv = get("multipv").and_then(|i| tok.get(i + 1)?.parse().ok()).unwrap_or(1);
    let depth = get("depth").and_then(|i| tok.get(i + 1)?.parse().ok()).unwrap_or(0);
    let num = |i: usize| -> Option<f64> { tok.get(i)?.parse().ok() };
    let wdl = Wdl::new(num(w + 1)?, num(w + 2)?, num(w + 3)?);
    let (mut cp, mut mate) = (None, None);
    if let Some(i) = get("score") {
        let v: Option<i32> = tok.get(i + 2).and_then(|t| t.parse().ok());
        match tok.get(i + 1) {
            Some(&"cp") => cp = v,
            Some(&"mate") => mate = v,
            _ => {}
        }
    }
    let bound = tok.contains(&"lowerbound") || tok.contains(&"upperbound");
    Some((mpv, EngineLine { uci: tok.get(pv + 1)?.to_string(), wdl, depth, cp, mate }, bound))
}

/// Collect a search's lines: per MultiPV index, the last exact line, else the last bound
/// line; then drop repeated moves (stale lines from an unfinished iteration).
pub fn collect_lines(lines: &[String]) -> Vec<EngineLine> {
    let mut slots: Vec<Slot> = Vec::new();
    for l in lines {
        if let Some((i, el, bound)) = parse_info(l) {
            if slots.len() < i {
                slots.resize_with(i, Slot::default);
            }
            if bound {
                slots[i - 1].bound = Some(el);
            } else {
                slots[i - 1].exact = Some(el);
            }
        }
    }
    let mut out: Vec<EngineLine> = Vec::new();
    for s in slots {
        if let Some(l) = s.exact.or(s.bound) {
            if !out.iter().any(|o| o.uci == l.uci) {
                out.push(l);
            }
        }
    }
    out
}

/// Largest piece count Stockfish loaded tablebases for, from its
/// "info string Found N WDL and M DTZ tablebase files (up to K-man)." line.
pub fn parse_tablebase_men(lines: &[String]) -> Option<u32> {
    lines.iter().find_map(|l| {
        let rest = l.split("(up to ").nth(1)?;
        rest.split('-').next()?.trim().parse().ok()
    })
}

impl Stockfish {
    /// Start one process. With `syzygy`, Stockfish probes the tablebases in that folder
    /// (several folders: separate with ';' on Windows, ':' elsewhere).
    pub fn start(path: &Path, hash_mb: u32, syzygy: Option<&Path>) -> anyhow::Result<Stockfish> {
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().with_context(|| format!("starting Stockfish at {}", path.display()))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut sf = Stockfish { child, stdin, stdout, multipv: 1, name: String::new() };
        sf.send("uci")?;
        for l in sf.read_until("uciok")? {
            if let Some(n) = l.strip_prefix("id name ") {
                sf.name = n.trim().to_string();
            }
        }
        sf.send("setoption name Threads value 1")?;
        sf.send(&format!("setoption name Hash value {hash_mb}"))?;
        sf.send("setoption name UCI_ShowWDL value true")?;
        if let Some(tb) = syzygy {
            sf.send(&format!("setoption name SyzygyPath value {}", tb.display()))?;
        }
        sf.send("isready")?;
        let lines = sf.read_until("readyok")?;
        // Only count tablebases as in use if Stockfish actually found some.
        if let Some(men) = parse_tablebase_men(&lines).filter(|&m| m > 0) {
            sf.name = format!("{} + Syzygy {men}-man", sf.name);
        }
        Ok(sf)
    }

    fn send(&mut self, s: &str) -> anyhow::Result<()> {
        writeln!(self.stdin, "{s}")?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_until(&mut self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let mut out = Vec::new();
        let mut buf = String::new();
        loop {
            buf.clear();
            if self.stdout.read_line(&mut buf)? == 0 {
                bail!("Stockfish exited unexpectedly");
            }
            let l = buf.trim_end().to_string();
            let done = l.starts_with(prefix);
            out.push(l);
            if done {
                return Ok(out);
            }
        }
    }

    pub fn search(&mut self, pos: &Chess, nodes: u64, multipv: usize) -> anyhow::Result<EngineResult> {
        self.send("ucinewgame")?;
        if multipv != self.multipv {
            self.send(&format!("setoption name MultiPV value {multipv}"))?;
            self.multipv = multipv;
        }
        self.send("isready")?;
        self.read_until("readyok")?;
        self.send(&format!("position fen {}", full_fen(pos)))?;
        self.send(&format!("go nodes {nodes}"))?;
        let lines = self.read_until("bestmove")?;
        let lines = collect_lines(&lines);
        if lines.is_empty() {
            bail!("Stockfish gave no lines for {}", full_fen(pos));
        }
        Ok(EngineResult { lines })
    }
}

impl Drop for Stockfish {
    fn drop(&mut self) {
        let _ = self.send("quit");
        let _ = self.child.wait();
    }
}

/// N single-threaded Stockfish processes; a batch of searches is spread across them.
pub struct StockfishPool {
    engines: Vec<Mutex<Stockfish>>,
    pub nodes: u64,
    pub hash_mb: u32,
    pub name: String,
    pub path: PathBuf,
}

impl StockfishPool {
    pub fn start(
        path: &Path,
        workers: usize,
        nodes: u64,
        hash_mb: u32,
        syzygy: Option<&Path>,
    ) -> anyhow::Result<StockfishPool> {
        let engines = (0..workers.max(1))
            .map(|_| Stockfish::start(path, hash_mb, syzygy).map(Mutex::new))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let name = engines[0].lock().unwrap().name.clone();
        Ok(StockfishPool { engines, nodes, hash_mb, name, path: path.to_path_buf() })
    }

    pub fn search_many(&self, positions: &[Chess], multipv: usize) -> anyhow::Result<Vec<EngineResult>> {
        let next = AtomicUsize::new(0);
        let results: Mutex<Vec<Option<anyhow::Result<EngineResult>>>> =
            Mutex::new((0..positions.len()).map(|_| None).collect());
        std::thread::scope(|scope| {
            for eng in self.engines.iter().take(positions.len()) {
                scope.spawn(|| {
                    let mut sf = eng.lock().unwrap();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= positions.len() {
                            break;
                        }
                        let r = sf.search(&positions[i], self.nodes, multipv);
                        results.lock().unwrap()[i] = Some(r);
                    }
                });
            }
        });
        results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|r| r.unwrap_or_else(|| Err(anyhow!("search not run"))))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_prefers_exact_lines_and_dedups() {
        let raw = [
            "info depth 11 seldepth 13 multipv 1 score cp 30 wdl 60 930 10 nodes 1 pv e2e4 e7e5",
            "info depth 11 seldepth 13 multipv 2 score cp 20 wdl 50 940 10 nodes 1 pv d2d4",
            "info depth 11 seldepth 13 multipv 3 score cp 10 wdl 40 950 10 nodes 1 pv c2c4",
            "info depth 12 seldepth 13 multipv 1 score cp 44 lowerbound wdl 92 905 3 nodes 9 pv c2c4",
            "info depth 12 seldepth 13 multipv 2 score cp 25 wdl 55 935 10 nodes 9 pv e2e4",
            "bestmove c2c4",
        ]
        .map(String::from);
        let l = collect_lines(&raw);
        // Slot 1 keeps its exact depth-11 line (e2e4); slot 2 is e2e4 at depth 12, a
        // duplicate, so it's dropped; slot 3 (c2c4) survives.
        let moves: Vec<_> = l.iter().map(|x| x.uci.as_str()).collect();
        assert_eq!(moves, ["e2e4", "c2c4"]);
        assert!((l[0].wdl.win - 0.06).abs() < 1e-12);
    }

    #[test]
    fn parse_tablebase_line() {
        let l = |s: &str| vec![s.to_string(), "readyok".to_string()];
        assert_eq!(parse_tablebase_men(&l("info string Found 145 WDL and 145 DTZ tablebase files (up to 5-man).")), Some(5));
        assert_eq!(parse_tablebase_men(&l("info string Found 0 WDL and 0 DTZ tablebase files (up to 0-man).")), Some(0));
        assert_eq!(parse_tablebase_men(&l("info string NNUE evaluation enabled")), None);
    }

    #[test]
    fn parse_ignores_lines_without_wdl_or_pv() {
        assert!(parse_info("info string NNUE evaluation using nn-xyz.nnue").is_none());
        assert!(parse_info("info depth 1 score cp 10 pv e2e4").is_none());
        let (i, l, b) = parse_info("info depth 5 score mate 2 wdl 1000 0 0 pv h5f7").unwrap();
        assert_eq!((i, l.uci.as_str(), b), (1, "h5f7", false));
        assert_eq!((l.mate, l.cp, l.pawns()), (Some(2), None, 12.0));
        let (_, l, _) = parse_info("info depth 9 multipv 1 score cp -137 wdl 20 600 380 pv e7e5").unwrap();
        assert_eq!((l.cp, l.pawns()), (Some(-137), -1.37));
    }
}
