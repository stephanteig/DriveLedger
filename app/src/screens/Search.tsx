import { useState, type FormEvent } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { compareApi, errorMessage, joinPath, type Page, type SearchHit } from "../lib/api";
import { bytes, count, dateFromNanos } from "../lib/format";
import { Badge, Button, Empty, ErrorNote, Icon, PageHeader, Panel } from "../components/ui";

export default function Search() {
  const [text, setText] = useState("");
  const [ext, setExt] = useState("");
  const [minSize, setMinSize] = useState("");
  const [result, setResult] = useState<Page<SearchHit> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async (e?: FormEvent, fingerprint?: string) => {
    e?.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const isGlob = /[*?[]/.test(text);
      const page = await compareApi.search(
        fingerprint
          ? { fingerprint }
          : {
              text: isGlob ? null : text || null,
              glob: isGlob ? text : null,
              extensions: ext ? ext.split(/[\s,]+/).filter(Boolean) : [],
              min_size: minSize ? Math.round(Number(minSize) * 1e6) : null,
            },
      );
      setResult(page);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const reveal = async (hit: SearchHit) => {
    setError(null);
    if (!(await compareApi.isAvailable(hit.root_path))) {
      setError(`Connect ${hit.volume_name} to reveal this file.`);
      return;
    }
    await revealItemInDir(joinPath(hit.root_path, hit.file.rel_path)).catch((e) => setError(errorMessage(e)));
  };

  // Group hits by content so every location of the same file is visible together.
  const copies = new Map<string, number>();
  result?.items.forEach((h) => {
    if (h.file.fingerprint) copies.set(h.file.fingerprint, (copies.get(h.file.fingerprint) ?? 0) + 1);
  });

  return (
    <>
      <PageHeader title="Search" subtitle="Across every saved index. Use * or ? for a glob on the file name." />
      <Panel className="mb-4">
        <form onSubmit={run} className="p-4 grid grid-cols-[1fr_160px_140px_auto] gap-3">
          <input
            autoFocus
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="SW-FX30157 or SW-FX3*.MP4 or 04_Partnere/Red-Bull"
            className="h-8 px-3 rounded-md border border-border bg-bg mono text-[12px] outline-none focus:border-cyan/60"
          />
          <input
            value={ext}
            onChange={(e) => setExt(e.target.value)}
            placeholder="ext: mp4, mov"
            className="h-8 px-3 rounded-md border border-border bg-bg mono text-[12px] outline-none focus:border-cyan/60"
          />
          <input
            value={minSize}
            onChange={(e) => setMinSize(e.target.value)}
            placeholder="min MB"
            inputMode="decimal"
            className="h-8 px-3 rounded-md border border-border bg-bg mono text-[12px] outline-none focus:border-cyan/60"
          />
          <Button variant="primary" className="h-8" disabled={busy}>
            <Icon name="search" /> {busy ? "Searching…" : "Search"}
          </Button>
        </form>
      </Panel>
      {error && <div className="mb-4"><ErrorNote>{error}</ErrorNote></div>}
      {result && (
        <Panel
          title={`Results · ${count(result.total)}${result.next_cursor ? ` (showing ${count(result.items.length)})` : ""}`}
        >
          {result.items.length === 0 ? (
            <Empty>No matches.</Empty>
          ) : (
            <div className="max-h-[640px] overflow-auto">
              <table className="w-full text-[12px]">
                <thead className="sticky top-0 bg-surface">
                  <tr className="text-left text-[10px] uppercase tracking-[0.12em] text-faint">
                    <th className="h-8 px-4">Volume</th>
                    <th className="h-8 px-3 text-right">Size</th>
                    <th className="h-8 px-3">Modified</th>
                    <th className="h-8 px-3">Path</th>
                    <th className="h-8 px-3">Copies</th>
                    <th className="h-8 px-4" />
                  </tr>
                </thead>
                <tbody>
                  {result.items.map((h) => {
                    const n = h.file.fingerprint ? copies.get(h.file.fingerprint) ?? 1 : 1;
                    return (
                      <tr key={`${h.index_id}/${h.file.rel_path}`} className="h-8 border-t border-border/60 hover:bg-surface-2/50">
                        <td className="px-4 font-medium whitespace-nowrap">{h.volume_name}</td>
                        <td className="px-3 text-right mono tabular whitespace-nowrap">{bytes(h.file.size)}</td>
                        <td className="px-3 mono tabular text-muted whitespace-nowrap">{dateFromNanos(h.file.mtime)}</td>
                        <td className="px-3 mono truncate max-w-[560px]" title={joinPath(h.root_path, h.file.rel_path_nfc)}>
                          <span className="text-faint">{h.root_path}/</span>
                          {h.file.rel_path_nfc}
                          {h.file.in_bundle && <span className="ml-2"><Badge tone="violet">bundle</Badge></span>}
                        </td>
                        <td className="px-3">
                          {h.file.fingerprint && (
                            <button
                              className="mono text-[11px] text-violet hover:underline"
                              title="Show every copy of this content"
                              onClick={() => run(undefined, h.file.fingerprint!)}
                            >
                              {n > 1 ? `${n} here · all` : "find all"}
                            </button>
                          )}
                        </td>
                        <td className="px-4 text-right">
                          <Button onClick={() => reveal(h)}>Reveal</Button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </Panel>
      )}
    </>
  );
}
