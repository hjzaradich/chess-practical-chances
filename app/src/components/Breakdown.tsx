import type { Diagnostics, PlyResult, Settings } from "../types";

interface Props {
  result: PlyResult | undefined;
  whiteName: string;
  blackName: string;
  /** Shown when there's no result for this position yet. */
  emptyText: string;
}

const pct = (x: number) => `${Math.round(x * 100)}%`;
/** Two decimals, without a "-0.00" for tiny negative values (T is reported raw). */
export const f2 = (x: number) => (Math.abs(x) < 0.005 ? 0 : x).toFixed(2);

/** Chess players' names for a move's cost in expected score, at δ, 2δ and 3δ. With the
 *  default δ = 0.05 these are Lichess's thresholds (0.1/0.2/0.3 in its −1..1 win-chance
 *  scale) on our 0..1 scale (DECISIONS 5.2). */
const severity = (cost: number, delta: number) =>
  cost >= 3 * delta ? "Blunder" : cost >= 2 * delta ? "Mistake" : "Inaccuracy";
const severityNote = (delta: number) =>
  `Inaccuracy ≥ ${delta}, mistake ≥ ${+(2 * delta).toFixed(2)}, blunder ≥ ${+(3 * delta).toFixed(2)}.`;

function SettingsLine({ s }: { s: Settings }) {
  return (
    <div className="settings-line">
      Settings: k={s.k} · p={s.p} · {s.nodes.toLocaleString()} nodes · ε={s.epsilon} · δ={s.delta} · draw={s.d} ·
      ratings {s.white_elo}/{s.black_elo} · {s.stockfish} · {s.maia}
    </div>
  );
}

function Side({
  color,
  name,
  elo,
  t,
  diag,
  delta,
}: {
  color: "white" | "black";
  name: string;
  elo: number;
  t: number;
  diag: Diagnostics | null;
  delta: number;
}) {
  return (
    <div className="side">
      <h4>
        {color === "white" ? "White" : "Black"}
        {name && <span className="elo">{name}</span>}
        <span className="elo">{elo}</span>
      </h4>
      <div className="stats">
        <div className="stat" title="Expected points lost over this player's next decisions, vs. the engine eval">
          <div className="label">Trickiness T</div>
          <div className="value">{f2(t)}</div>
        </div>
        <div className="stat" title="Moves within ε of the best (Stockfish MultiPV)">
          <div className="label">Stable moves</div>
          <div className="value">{diag ? diag.stable_moves : "–"}</div>
        </div>
        <div className="stat" title={`Maia's probability on likely moves that are inaccuracies or worse (give up more than ${delta} of expected score)`}>
          <div className="label">Trap mass</div>
          <div className="value">{diag ? pct(diag.trap_mass) : "–"}</div>
        </div>
      </div>
      {!diag ? (
        <div className="where muted">The game ends before this player moves.</div>
      ) : (
        <>
          <div className="where">
            {diag.at_ply_offset === 0
              ? "To move now. Likely moves:"
              : diag.via
                ? `Decides after ${diag.via}, the opponent's best reply. Likely moves there:`
                : "Decides after the opponent's best reply. Likely moves there:"}
          </div>
          <table className="moves">
            <thead>
              <tr>
                <th>Move</th>
                <th>Chance ({elo})</th>
                <th className="num">Value</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {diag.moves.map((m) => (
                <tr key={m.uci} className={m.loses ? "loses" : ""}>
                  <td>{m.san}</td>
                  <td>
                    <span className="pbar" style={{ width: `${Math.max(2, m.p * 60)}px` }} />
                    {pct(m.p)}
                  </td>
                  <td className="num">{f2(m.v)}</td>
                  <td className="num">
                    {m.loses && (
                      <span
                        className="flag"
                        title={`Gives up ${(diag.best_value - m.v).toFixed(2)} of expected score vs. the best move. ${severityNote(delta)}`}
                      >
                        ⚠ {severity(diag.best_value - m.v, delta)} −{(diag.best_value - m.v).toFixed(2)}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="where muted" style={{ marginTop: 4 }}>
            Value = expected score for {color === "white" ? "White" : "Black"} after the move. Best move: {f2(diag.best_value)}.
          </div>
        </>
      )}
    </div>
  );
}

export function Breakdown({ result, whiteName, blackName, emptyText }: Props) {
  if (!result) {
    return (
      <div className="panel breakdown">
        <h3>Position breakdown</h3>
        <div className="muted">{emptyText}</div>
      </div>
    );
  }
  const s = result.settings;
  return (
    <div className="panel breakdown">
      <h3>
        Position breakdown · eval {f2(result.v_star)} for White · draw rate {pct(result.draw_rate)}
      </h3>
      <div className="sides">
        <Side color="white" name={whiteName} elo={s.white_elo} t={result.trickiness.white} diag={result.diagnostics.white} delta={s.delta} />
        <Side color="black" name={blackName} elo={s.black_elo} t={result.trickiness.black} diag={result.diagnostics.black} delta={s.delta} />
      </div>
      <SettingsLine s={s} />
    </div>
  );
}
