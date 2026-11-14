import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, errorText, events } from "./api";
import { Board, PromotionPicker } from "./components/Board";
import { Breakdown } from "./components/Breakdown";
import { GameChart } from "./components/GameChart";
import { MoveList } from "./components/MoveList";
import { PasteDialog, Setup } from "./components/Setup";
import { PracticalPanel } from "./components/PracticalPanel";
import { SideLine } from "./components/SideLine";
import { PRESETS, type GameInfo, type MoveInfo, type PlyResult, type Practical, type Preset, type Status } from "./types";

const START_FEN = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

type RunState =
  | { kind: "idle" }
  | { kind: "running"; started: number }
  | { kind: "cancelling" }
  | { kind: "done"; seconds: number; fromSaved: boolean }
  | { kind: "cancelled" }
  | { kind: "error"; message: string };

/** One side line (or the new board), analysed position by position on demand. Discarded on close. */
interface Line {
  id: number;
  /** True for a side line branched from the loaded game; false for the new board. */
  fromGame: boolean;
  /** Half-moves played before the line starts. */
  basePly: number;
  title: string;
  closeLabel: string;
  /** FENs oldest..base position, for Maia's move history. */
  baseHist: string[];
  baseLastMove: [string, string] | null;
  moves: MoveInfo[];
  /** 0 = the base position, i = after moves[i - 1]. */
  cursor: number;
  results: (PlyResult | undefined)[];
  errors: (string | undefined)[];
}

function moveLabel(ply: number, san: string) {
  const n = Math.ceil(ply / 2);
  return `${n}${ply % 2 ? "." : "..."} ${san}`;
}

export function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [pgn, setPgn] = useState("");
  const [games, setGames] = useState<GameInfo[]>([]);
  const [gameIndex, setGameIndex] = useState(0);
  const [whiteElo, setWhiteElo] = useState(1500);
  const [blackElo, setBlackElo] = useState(1500);
  const [preset, setPreset] = useState<Preset>("standard");
  const [plies, setPlies] = useState<(PlyResult | undefined)[]>([]);
  const [current, setCurrent] = useState(0);
  const [orientation, setOrientation] = useState<"white" | "black">("white");
  const [run, setRun] = useState<RunState>({ kind: "idle" });
  const [pasting, setPasting] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const [line, setLine] = useState<Line | null>(null);
  const [legal, setLegal] = useState<string[]>([]);
  const [promo, setPromo] = useState<{ base: string; color: "white" | "black" } | null>(null);
  // Phase 4: practical move, per position (+ draw value + settings), computed on request.
  const [drawValue, setDrawValue] = useState(0.5);
  const [practical, setPractical] = useState<Record<string, Practical>>({});
  const [practicalErr, setPracticalErr] = useState<Record<string, string>>({});
  const [practicalAsked, setPracticalAsked] = useState<Set<string>>(new Set());
  const [practicalBusy, setPracticalBusy] = useState<string | null>(null);
  const startedRef = useRef(0);
  const lineIds = useRef(0);
  const inFlight = useRef("");

  const game = games[gameIndex];
  const nPlies = game ? game.moves.length + 1 : 0;
  const done = plies.filter(Boolean).length;
  const running = run.kind === "running" || run.kind === "cancelling";

  const refreshStatus = useCallback(() => api.status().then(setStatus), []);
  useEffect(() => void refreshStatus(), [refreshStatus]);

  // Stream: one event per ply, the chart fills as they arrive.
  useEffect(() => {
    const a = events.onPly((r) =>
      setPlies((prev) => {
        const next = prev.slice();
        next[r.ply] = r;
        return next;
      }),
    );
    const b = events.onDone((d) => {
      if (d.status === "done") setRun({ kind: "done", seconds: (Date.now() - startedRef.current) / 1000, fromSaved: d.from_saved });
      else if (d.status === "cancelled") setRun({ kind: "cancelled" });
      else setRun({ kind: "error", message: d.message ?? "unknown error" });
    });
    return () => {
      a.then((f) => f());
      b.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (run.kind !== "running") return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [run.kind]);

  function selectGame(list: GameInfo[], i: number) {
    const g = list[i];
    setGameIndex(i);
    setWhiteElo(Math.round(g?.white_elo ?? 1500));
    setBlackElo(Math.round(g?.black_elo ?? 1500));
    setPlies([]);
    setCurrent(0);
    setLine(null);
    setRun({ kind: "idle" });
  }

  async function loadText(text: string) {
    setLoadError(null);
    try {
      const list = await api.parsePgn(text);
      const first = list.findIndex((g) => !g.error);
      if (first < 0) throw new Error(list[0]?.error ?? "No readable games in this PGN.");
      setPgn(text);
      setGames(list);
      selectGame(list, first);
    } catch (e) {
      setLoadError(errorText(e));
    }
  }

  async function openFile() {
    const path = await open({ multiple: false, filters: [{ name: "PGN", extensions: ["pgn", "txt"] }] });
    if (path) loadText(await api.readTextFile(path as string));
  }

  async function analyze() {
    if (!game) return;
    setPlies([]);
    startedRef.current = Date.now();
    setNow(Date.now());
    setRun({ kind: "running", started: Date.now() });
    try {
      await api.startAnalysis({ pgn, game_index: gameIndex, white_elo: whiteElo, black_elo: blackElo, preset });
    } catch (e) {
      setRun({ kind: "error", message: errorText(e) });
    }
  }

  async function cancel() {
    setRun({ kind: "cancelling" });
    await api.cancelAnalysis();
  }

  async function exportAs(format: "json" | "csv") {
    const rows = plies.filter(Boolean) as PlyResult[];
    const base = game ? `${game.white || "white"}-${game.black || "black"}`.replace(/[^\w.-]+/g, "_") : "game";
    const path = await save({ defaultPath: `${base}.trickiness.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
    if (!path) return;
    try {
      await api.exportResults(path, format, rows);
    } catch (e) {
      setLoadError(errorText(e));
    }
  }

  // ------------------------------------------------------------ current position

  const gameFens = useMemo(() => (game ? [game.start_fen, ...game.moves.map((m) => m.fen)] : []), [game]);
  const fen = line
    ? line.cursor === 0 ? line.baseHist[line.baseHist.length - 1] : line.moves[line.cursor - 1].fen
    : game ? gameFens[current] : START_FEN;
  const lastMove = line
    ? line.cursor === 0 ? line.baseLastMove?.[1] ?? null : line.moves[line.cursor - 1].uci
    : game && current > 0 ? game.moves[current - 1].uci : null;
  const result = line ? line.results[line.cursor] : plies[current];
  // History of the position on the board (oldest..current), for Maia.
  const currentHist = useMemo(
    () =>
      (line
        ? [...line.baseHist, ...line.moves.slice(0, line.cursor).map((m) => m.fen)]
        : game ? gameFens.slice(0, current + 1) : [START_FEN]
      ).slice(-8),
    [line, game, gameFens, current],
  );
  const arrows = useMemo(() => result?.likely_moves ?? [], [result]);
  const turn: "white" | "black" = fen.split(" ")[1] === "b" ? "black" : "white";

  // Legal moves come from the core (no chess rules in the front end).
  useEffect(() => {
    let live = true;
    api.legalMoves(fen).then((m) => live && setLegal(m), () => live && setLegal([]));
    return () => {
      live = false;
    };
  }, [fen]);

  // ------------------------------------------------------------ side line / new board

  function openLine(fromGame: boolean, basePly: number, baseHist: string[], baseLastMove: [string, string] | null, title: string, closeLabel: string, first?: MoveInfo, baseResult?: PlyResult): Line {
    return {
      id: ++lineIds.current,
      fromGame,
      basePly,
      title,
      closeLabel,
      baseHist: baseHist.slice(-8),
      baseLastMove,
      moves: first ? [first] : [],
      cursor: first ? 1 : 0,
      results: [baseResult],
      errors: [],
    };
  }

  function newBoard() {
    setLine(openLine(false, 0, [START_FEN], null, "Board from the starting position", game ? "Back to game" : "Close board"));
  }

  async function commit(uci: string) {
    let mv: MoveInfo;
    try {
      mv = await api.playMove(fen, uci);
    } catch (e) {
      setLegal((l) => l.slice()); // resets the board to the current position
      return;
    }
    if (line) {
      setLine((l) => {
        if (!l) return l;
        if (l.moves[l.cursor]?.uci === uci) return { ...l, cursor: l.cursor + 1 }; // same as the line's next move
        return {
          ...l,
          moves: [...l.moves.slice(0, l.cursor), mv],
          cursor: l.cursor + 1,
          results: l.results.slice(0, l.cursor + 1),
          errors: l.errors.slice(0, l.cursor + 1),
        };
      });
    } else if (game && current < nPlies - 1 && game.moves[current].uci === uci) {
      setCurrent(current + 1); // following the game
    } else if (game) {
      const last: [string, string] | null = current > 0 ? [game.moves[current - 1].san, game.moves[current - 1].uci] : null;
      const title = current === 0 ? "Side line from the start" : `Side line after ${moveLabel(current, game.moves[current - 1].san)}`;
      setLine(openLine(true, current, gameFens.slice(0, current + 1), last, title, "Back to game", mv, plies[current]));
    }
  }

  function onBoardMove(orig: string, dest: string) {
    const cands = legal.filter((u) => u.startsWith(orig + dest));
    if (!cands.length) return;
    if (cands.some((u) => u.length === 5)) setPromo({ base: orig + dest, color: turn });
    else commit(cands[0]);
  }

  // Changing ratings or preset invalidates the line's numbers.
  const settingsKey = `${whiteElo}/${blackElo}/${preset}`;
  const settingsRef = useRef(settingsKey);
  useEffect(() => {
    settingsRef.current = settingsKey;
    setLine((l) => (l ? { ...l, results: [], errors: [] } : l));
  }, [settingsKey]);

  // Analyse the line position on screen, once. A newer request supersedes an older one.
  useEffect(() => {
    if (!line || !status?.model_path) return;
    const i = line.cursor;
    if (line.results[i] || line.errors[i]) return;
    const id = line.id;
    const key = settingsKey;
    const flight = `${id}:${i}:${line.moves[i - 1]?.uci ?? ""}:${key}`;
    if (inFlight.current === flight) return; // already asked for this exact position
    inFlight.current = flight;
    const hist = [...line.baseHist, ...line.moves.slice(0, i).map((m) => m.fen)].slice(-8);
    const last: [string, string] | null = i > 0 ? [line.moves[i - 1].san, line.moves[i - 1].uci] : line.baseLastMove;
    api
      .analyzePosition({ history: hist, ply: line.basePly + i, last_move: last, white_elo: whiteElo, black_elo: blackElo, preset })
      .then((r) =>
        setLine((l) => {
          if (!l || l.id !== id || settingsRef.current !== key || l.moves[i - 1]?.fen !== line.moves[i - 1]?.fen) return l;
          const results = l.results.slice();
          results[i] = r;
          return { ...l, results };
        }),
      )
      .finally(() => {
        if (inFlight.current === flight) inFlight.current = "";
      })
      .catch((e) => {
        const msg = errorText(e);
        if (msg.includes("superseded")) return;
        setLine((l) => {
          if (!l || l.id !== id) return l;
          const errors = l.errors.slice();
          errors[i] = msg;
          return { ...l, errors };
        });
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [line?.id, line?.cursor, line?.moves, line?.results, settingsKey, status?.model_path]);

  // ------------------------------------------------------------ practical move (Phase 4)

  const posKey = `${currentHist.join("|")}#${settingsKey}`;
  const practicalKey = `${posKey}#${drawValue}`;
  const practicalResult = practical[practicalKey];
  useEffect(() => {
    if (!practicalAsked.has(posKey) || practical[practicalKey] || practicalErr[practicalKey] || practicalBusy === practicalKey) return;
    if (!status?.model_path) return;
    const key = practicalKey;
    setPracticalBusy(key);
    api
      .analyzePractical({ history: currentHist, white_elo: whiteElo, black_elo: blackElo, preset, d: drawValue })
      .then((r) => setPractical((m) => ({ ...m, [key]: r })))
      .catch((e) => {
        const msg = errorText(e);
        if (!msg.includes("superseded")) setPracticalErr((m) => ({ ...m, [key]: msg }));
      })
      .finally(() => setPracticalBusy((b) => (b === key ? null : b)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [practicalKey, practicalAsked, status?.model_path]);

  // ------------------------------------------------------------ navigation

  const step = useCallback(
    (to: "start" | "back" | "forward" | "end") => {
      if (line) {
        setLine((l) => {
          if (!l) return l;
          const c = to === "start" ? 0 : to === "end" ? l.moves.length : to === "back" ? Math.max(0, l.cursor - 1) : Math.min(l.moves.length, l.cursor + 1);
          return { ...l, cursor: c };
        });
      } else if (nPlies) {
        setCurrent((c) => (to === "start" ? 0 : to === "end" ? nPlies - 1 : to === "back" ? Math.max(0, c - 1) : Math.min(nPlies - 1, c + 1)));
      }
    },
    [line, nPlies],
  );

  // Keyboard: ← → step; Home/End jump; Esc leaves the side line.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if (e.key === "ArrowLeft") step("back");
      else if (e.key === "ArrowRight") step("forward");
      else if (e.key === "Home") step("start");
      else if (e.key === "End") step("end");
      else if (e.key === "Escape" && line) setLine(null);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [step, line]);

  const selectGamePly = (ply: number) => {
    setLine(null);
    setCurrent(ply);
  };

  const needsSetup = status && (!status.model_path || !status.stockfish_path);
  const elapsed = run.kind === "running" ? Math.round((now - run.started) / 1000) : 0;
  const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, "0")}`;
  const lineError = line ? line.errors[line.cursor] : undefined;
  const breakdownEmpty = line
    ? lineError
      ? `Couldn't analyse this position: ${lineError}`
      : "Analysing this position…"
    : running
      ? "This ply isn't analysed yet."
      : "Analyse the game to see the breakdown.";

  return (
    <div className="app">
      <div className="toolbar">
        <div className="group">
          <button onClick={openFile} disabled={running}>Open PGN…</button>
          <button onClick={() => setPasting(true)} disabled={running}>Paste PGN</button>
          <button onClick={newBoard} title="A board from the starting position, to play through a game or try lines">New board</button>
        </div>
        {games.length > 1 && (
          <select value={gameIndex} disabled={running} onChange={(e) => selectGame(games, Number(e.target.value))} aria-label="Game">
            {games.map((g) => (
              <option key={g.index} value={g.index} disabled={!!g.error}>
                {g.index + 1}. {g.error ? `(unreadable: ${g.error})` : `${g.white} – ${g.black} ${g.date}`}
              </option>
            ))}
          </select>
        )}
        <div className="game-title" title={game ? `${game.white} – ${game.black} · ${[game.event, game.date, game.result].filter(Boolean).join(" · ")}` : ""}>
          {game && (
            <>
              {game.white || "White"} – {game.black || "Black"}
              <span className="sub">{[game.event, game.date, game.result].filter(Boolean).join(" · ")}</span>
            </>
          )}
        </div>
        <div className="group">
          <label htmlFor="we">White</label>
          <input id="we" type="number" min={0} max={4000} step={10} value={whiteElo} disabled={running} onChange={(e) => setWhiteElo(Number(e.target.value))} />
          <label htmlFor="be">Black</label>
          <input id="be" type="number" min={0} max={4000} step={10} value={blackElo} disabled={running} onChange={(e) => setBlackElo(Number(e.target.value))} />
        </div>
        <div className="group">
          <select value={preset} disabled={running} onChange={(e) => setPreset(e.target.value as Preset)} aria-label="Preset"
            title={PRESETS.find((p) => p.id === preset)?.detail}>
            {PRESETS.map((p) => (
              <option key={p.id} value={p.id}>{p.label} ({p.detail})</option>
            ))}
          </select>
        </div>
        <div className="group">
          {running ? (
            <button className="danger" onClick={cancel} disabled={run.kind === "cancelling"}>
              {run.kind === "cancelling" ? "Cancelling…" : "Cancel"}
            </button>
          ) : (
            <button className="primary" onClick={analyze} disabled={!game || !!needsSetup}>Analyse</button>
          )}
          <button onClick={() => exportAs("json")} disabled={!done || running}>Export JSON</button>
          <button onClick={() => exportAs("csv")} disabled={!done || running}>Export CSV</button>
        </div>
      </div>

      {(run.kind !== "idle" || loadError) && (
        <div className="progress">
          {loadError && <span className="error">{loadError}</span>}
          {run.kind === "running" && (
            <>
              <div className="bar"><div style={{ width: `${(100 * done) / Math.max(1, nPlies)}%` }} /></div>
              Analysing ply {Math.min(done + 1, nPlies)} of {nPlies} · {fmt(elapsed)}
              {done > 2 && ` · about ${fmt(((nPlies - done) * elapsed) / done)} left`}
            </>
          )}
          {run.kind === "cancelling" && <>Cancelling after the current step…</>}
          {run.kind === "done" && (run.fromSaved ? <>Loaded saved results ({nPlies} plies).</> : <>Done: {nPlies} plies in {fmt(run.seconds)}.</>)}
          {run.kind === "cancelled" && <>Cancelled after {done} of {nPlies} plies.</>}
          {run.kind === "error" && <span className="error">Analysis failed: {run.message}</span>}
        </div>
      )}

      <div className="main">
        {!game && !line ? (
          <div className="empty-state">
            <div>
              <h2>Open or paste a PGN to begin</h2>
              <p>The app shows the engine eval, the draw rate, and how tricky each move is for each player at their rating.</p>
              <p><button onClick={newBoard}>Or start from a new board</button></p>
            </div>
          </div>
        ) : (
          <>
            <div className="board-col">
              <Board fen={fen} lastMove={lastMove} orientation={orientation} arrows={arrows} legal={legal} onMove={onBoardMove}
                practicalMove={practicalResult?.clear ? practicalResult.candidates[practicalResult.practical_best].uci : null} />
              <div className="board-nav">
                <button onClick={() => step("start")} aria-label="Start">⏮</button>
                <button onClick={() => step("back")} aria-label="Back">◀</button>
                <button onClick={() => step("forward")} aria-label="Forward">▶</button>
                <button onClick={() => step("end")} aria-label="End">⏭</button>
                <button onClick={() => setOrientation((o) => (o === "white" ? "black" : "white"))} aria-label="Flip board">⇅</button>
              </div>
            </div>
            <div className="move-col">
              {line && (
                <SideLine
                  basePly={line.basePly}
                  title={line.title}
                  moves={line.moves}
                  cursor={line.cursor}
                  onSelect={(i) => setLine((l) => (l ? { ...l, cursor: i } : l))}
                  onClose={() => setLine(null)}
                  closeLabel={line.closeLabel}
                />
              )}
              {game && (!line || line.fromGame) && <MoveList moves={game.moves} plies={plies} current={line ? -1 : current} onSelect={selectGamePly} />}
            </div>
            <div className="right-col">
              {game && (!line || line.fromGame) && <GameChart plies={plies} moves={game.moves} current={line ? line.basePly : current} onSelect={selectGamePly} />}
              <Breakdown result={result} whiteName={line && !line.fromGame ? "" : game?.white ?? ""} blackName={line && !line.fromGame ? "" : game?.black ?? ""} emptyText={breakdownEmpty} />
              <PracticalPanel
                side={turn}
                elo={turn === "white" ? whiteElo : blackElo}
                oppElo={turn === "white" ? blackElo : whiteElo}
                result={practicalResult}
                busy={practicalBusy === practicalKey}
                error={practicalErr[practicalKey] ?? null}
                d={drawValue}
                onD={setDrawValue}
                onRun={() => setPracticalAsked((s) => new Set(s).add(posKey))}
                gameOver={legal.length === 0}
              />
            </div>
          </>
        )}
      </div>

      {promo && (
        <PromotionPicker
          color={promo.color}
          onCancel={() => {
            setPromo(null);
            setLegal((l) => l.slice());
          }}
          onPick={(p) => {
            const uci = promo.base + p;
            setPromo(null);
            commit(uci);
          }}
        />
      )}
      {pasting && (
        <PasteDialog
          onClose={() => setPasting(false)}
          onLoad={(t) => {
            setPasting(false);
            loadText(t);
          }}
        />
      )}
      {needsSetup && status && <Setup status={status} onReady={refreshStatus} />}
    </div>
  );
}
