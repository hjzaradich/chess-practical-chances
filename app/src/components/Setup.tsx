import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorText, events } from "../api";
import type { Status } from "../types";

/** First run: the Maia model (and a check that Stockfish is bundled). */
export function Setup({ status, onReady }: { status: Status; onReady: () => void }) {
  const [size, setSize] = useState<number | null>(null);
  const [sizeError, setSizeError] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!status.download_url) return;
    api.modelDownloadSize().then(setSize, (e) => setSizeError(errorText(e)));
  }, [status.download_url]);

  useEffect(() => {
    const un = events.onModelProgress(setProgress);
    return () => void un.then((f) => f());
  }, []);

  const mb = (b: number) => `${(b / 1024 / 1024).toFixed(1)} MB`;

  async function download() {
    setBusy(true);
    setError(null);
    try {
      await api.downloadModel();
      onReady();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  async function locate() {
    const path = await open({ multiple: false, filters: [{ name: "Maia ONNX model", extensions: ["onnx"] }] });
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      await api.importModel(path as string);
      onReady();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="overlay">
      <div className="dialog">
        <h2>One-time setup</h2>
        {!status.stockfish_path && (
          <p className="error">Stockfish wasn't found next to the app. Reinstall, or set TRICKINESS_STOCKFISH.</p>
        )}
        {!status.model_path && (
          <>
            <p>
              The app needs the Maia-3 model (Maia3-5M, our ONNX export with move history) to predict human moves.
              It's stored in {status.data_dir}.
            </p>
            {status.download_url ? (
              <p>
                Download size: <b>{size !== null ? mb(size) : sizeError ? "unknown" : "checking…"}</b>
                {sizeError && <span className="error"> ({sizeError})</span>}
                {progress && progress.total > 0 && (
                  <> · {mb(progress.done)} of {mb(progress.total)}</>
                )}
              </p>
            ) : (
              <p className="muted">
                The download location isn't set up yet. Pick the model file instead
                (models/maia3-5m.history.fp32.onnx, made by tools/export_maia_onnx.py).
              </p>
            )}
            {error && <p className="error">{error}</p>}
            <div className="actions">
              <button onClick={locate} disabled={busy}>Locate model file…</button>
              {status.download_url && (
                <button className="primary" onClick={download} disabled={busy || size === null}>
                  {busy && progress ? "Downloading…" : `Download${size !== null ? ` (${mb(size)})` : ""}`}
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export function PasteDialog({ onLoad, onClose }: { onLoad: (text: string) => void; onClose: () => void }) {
  const [text, setText] = useState("");
  return (
    <div className="overlay" onClick={onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <h2>Paste a PGN</h2>
        <textarea autoFocus value={text} onChange={(e) => setText(e.target.value)} placeholder='[White "..."]&#10;&#10;1. e4 e5 2. Nf3 ...' />
        <div className="actions">
          <button onClick={onClose}>Cancel</button>
          <button className="primary" disabled={!text.trim()} onClick={() => onLoad(text)}>Load</button>
        </div>
      </div>
    </div>
  );
}
