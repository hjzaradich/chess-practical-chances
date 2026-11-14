import type { Practical } from "../types";
import { f2 } from "./Breakdown";

interface Props {
  /** The side to move at the position on the board, and its rating. */
  side: "white" | "black";
  elo: number;
  oppElo: number;
  result: Practical | undefined;
  busy: boolean;
  error: string | null;
  d: number;
  onD: (d: number) => void;
  onRun: () => void;
  gameOver: boolean;
}

const pct = (x: number) => `${Math.round(x * 100)}%`;
const cap = (s: string) => s[0].toUpperCase() + s.slice(1);

/** Phase 4: the objective best move next to the practical best move against this opponent. */
export function PracticalPanel({ side, elo, oppElo, result, busy, error, d, onD, onRun, gameOver }: Props) {
  const opp = side === "white" ? "Black" : "White";
  const ob = result ? result.candidates[result.objective_best] : undefined;
  const pb = result ? result.candidates[result.practical_best] : undefined;
  return (
    <div className="panel practical">
      <div className="practical-head">
        <h3>Practical move for {cap(side)} ({elo}) against {opp} ({oppElo})</h3>
        <label className="dslider" title="0.5 normally; 0 when only a win will do (the search then prefers riskier moves)">
          Draw is worth
          <input type="range" min={0} max={1} step={0.05} value={d} onChange={(e) => onD(Number(e.target.value))} aria-label="Value of a draw" />
          <b>{d.toFixed(2)}</b>
          {d === 0 && <span className="muted">(must win)</span>}
        </label>
        {!result && !busy && (
          <button className="primary" onClick={onRun} disabled={gameOver}>Find the practical move</button>
        )}
      </div>
      {busy && <div className="muted">Thinking about the candidates… (about 15 seconds at Standard)</div>}
      {error && <div className="error">Couldn't compute it: {error}</div>}
      {gameOver && <div className="muted">The game is over in this position.</div>}
      {result && ob && pb && (
        <>
          <div className="verdict">
            <div>Objective best: <b>{ob.san}</b> ({f2(ob.objective)}).</div>
            {pb.uci === ob.uci ? (
              <div>Practical best: <b>the same move</b> ({f2(pb.practical)} against this opponent).</div>
            ) : !result.clear ? (
              <div>
                Practical best: <b>no clear edge</b> over {ob.san}. {pb.san} gains only {f2(result.edge)} against this
                opponent, below the {result.params.margin} margin.
              </div>
            ) : (
              <>
                <div>
                  Practical best: <b>{pb.san}</b> ({f2(pb.practical)}, +{f2(result.edge)} over {ob.san} against this opponent).
                </div>
                {pb.best_reply && (
                  <div>
                    Relies on: {opp} missing <b>{pb.best_reply}</b>, which Maia says {oppElo}s miss {pct(pb.miss_chance)} of the time.
                    If they find it: {f2(pb.objective)}.
                  </div>
                )}
              </>
            )}
          </div>
          <table className="moves">
            <thead>
              <tr>
                <th>Move</th>
                <th className="num" title="Stockfish's value for you: the opponent replies perfectly">Objective</th>
                <th className="num" title="Value against this opponent (Q)">Practical</th>
                <th className="num" title="Chance the opponent misses the best reply (Maia)">They miss</th>
                <th className="num" title={`How often a ${elo} plays it (Maia)`}>You'd play</th>
              </tr>
            </thead>
            <tbody>
              {result.candidates.map((c, i) => (
                <tr key={c.uci} className={i === result.practical_best && result.clear ? "pbest" : ""}>
                  <td>
                    {c.san}
                    {i === result.objective_best && <span className="tag">engine</span>}
                    {i === result.practical_best && result.clear && <span className="tag practical-tag">practical</span>}
                  </td>
                  <td className="num">{f2(c.objective)}</td>
                  <td className="num">{f2(c.practical)}</td>
                  <td className="num">{c.replies.length ? pct(c.miss_chance) : "–"}</td>
                  <td className="num">{pct(c.p)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="settings-line">
            Practical settings: λ={result.params.lambda} (trust in the engine's reply) · τ={result.params.tau} (your later moves) ·
            horizon {result.params.horizon} · margin {result.params.margin} · p={result.params.p} · draw={result.params.d} · δ={result.params.delta} ·
            leaves: {result.leaf_model ?? result.params.leaf}
          </div>
        </>
      )}
    </div>
  );
}
