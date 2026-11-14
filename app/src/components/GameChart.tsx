import { CartesianGrid, Line, LineChart, ReferenceLine, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type { MoveInfo, PlyResult } from "../types";
import { f2 } from "./Breakdown";

interface Props {
  plies: (PlyResult | undefined)[];
  moves: MoveInfo[];
  current: number;
  onSelect: (ply: number) => void;
}

// Fixed categorical order (styles.css): v*, D, T White, T Black.
const SERIES = [
  { key: "v", label: "Eval v* (White's expected score)", color: "var(--series-1)" },
  { key: "d", label: "Draw rate D", color: "var(--series-2)" },
  { key: "tw", label: "T White", color: "var(--series-3)" },
  { key: "tb", label: "T Black", color: "var(--series-4)" },
] as const;

function moveLabel(ply: number, moves: MoveInfo[]) {
  if (ply === 0) return "Start";
  const n = Math.ceil(ply / 2);
  return `${n}${ply % 2 ? "." : "..."} ${moves[ply - 1]?.san ?? ""}`;
}

export function GameChart({ plies, moves, current, onSelect }: Props) {
  const data = Array.from({ length: moves.length + 1 }, (_, ply) => {
    const r = plies[ply];
    return {
      ply,
      v: r?.v_star ?? null,
      d: r?.draw_rate ?? null,
      tw: r?.trickiness.white ?? null,
      tb: r?.trickiness.black ?? null,
    };
  });
  const minT = Math.min(0, ...data.flatMap((x) => [x.tw ?? 0, x.tb ?? 0]));
  const any = plies.some(Boolean);

  return (
    <div className="panel chart">
      <h3>Game overview</h3>
      <div className="legend" aria-label="Legend">
        {SERIES.map((s) => (
          <span className="key" key={s.key}>
            <span className="swatch" style={{ background: s.color }} />
            {s.label}
          </span>
        ))}
      </div>
      {!any ? (
        <div className="chart-empty">Run an analysis to fill the chart. Plies appear as they finish.</div>
      ) : (
        <ResponsiveContainer width="100%" height={270}>
          <LineChart
            data={data}
            margin={{ top: 8, right: 12, bottom: 4, left: 0 }}
            onClick={(e) => {
              const ply = Number(e?.activeLabel ?? e?.activeTooltipIndex);
              if (Number.isFinite(ply)) onSelect(ply);
            }}
            style={{ cursor: "pointer" }}
          >
            <CartesianGrid stroke="var(--grid)" vertical={false} />
            <XAxis
              dataKey="ply"
              type="number"
              domain={[0, moves.length]}
              ticks={Array.from({ length: Math.floor(moves.length / 20) + 1 }, (_, i) => i * 20)}
              tickFormatter={(p: number) => String(p / 2 + 1)}
              stroke="var(--text-muted)"
              tick={{ fontSize: 11, fill: "var(--text-muted)" }}
              label={{ value: "move", position: "insideBottomRight", offset: -2, fontSize: 11, fill: "var(--text-muted)" }}
              allowDecimals={false}
            />
            <YAxis
              domain={[Math.floor((minT - 0.02) * 10) / 10, 1]}
              ticks={[0, 0.25, 0.5, 0.75, 1]}
              stroke="var(--text-muted)"
              tick={{ fontSize: 11, fill: "var(--text-muted)" }}
              width={36}
            />
            <ReferenceLine y={0.5} stroke="var(--grid)" strokeDasharray="4 4" />
            <ReferenceLine x={current} stroke="var(--text-secondary)" strokeWidth={1} />
            <Tooltip
              isAnimationActive={false}
              content={({ active, label }) => {
                const ply = Number(label);
                const r = plies[ply];
                if (!active || !r) return null;
                return (
                  <div className="tooltip">
                    <div className="t">{moveLabel(ply, moves)}</div>
                    <div className="r">Eval v* <b>{f2(r.v_star)}</b></div>
                    <div className="r">Draw rate <b>{f2(r.draw_rate)}</b></div>
                    <div className="r">T White <b>{f2(r.trickiness.white)}</b></div>
                    <div className="r">T Black <b>{f2(r.trickiness.black)}</b></div>
                  </div>
                );
              }}
            />
            {SERIES.map((s) => (
              <Line
                key={s.key}
                dataKey={s.key}
                stroke={s.color}
                strokeWidth={2}
                dot={false}
                activeDot={{ r: 4, stroke: "var(--surface-1)", strokeWidth: 2 }}
                isAnimationActive={false}
                connectNulls={false}
              />
            ))}
          </LineChart>
        </ResponsiveContainer>
      )}
    </div>
  );
}
