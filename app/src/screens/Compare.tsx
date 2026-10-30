import { useEffect, useMemo, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  api,
  compareApi,
  errorMessage,
  indexIdFromPath,
  joinPath,
  STATUS_LABEL,
  STATUS_TONE,
  STATUSES,
  type CommandError,
  type CompareResult,
  type FileComparison,
  type IndexSummary,
  type Status,
  type VerifyItem,
} from "../lib/api";
import { bytes, count, date } from "../lib/format";
import { useScans } from "../lib/scans";
import { Badge, Button, Empty, ErrorNote, Icon, PageHeader, Panel, toneText } from "../components/ui";

type Side = "source" | "target";

export default function Compare() {
  const { start, jobs, generation } = useScans();
  const [indexes, setIndexes] = useState<IndexSummary[]>([]);
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [subpath, setSubpath] = useState("");
  const [result, setResult] = useState<CompareResult | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const [pendingScan, setPendingScan] = useState<{ side: Side; id: string } | null>(null);

  useEffect(() => {
    api.listIndexes().then(setIndexes).catch(() => {});
  }, [generation]);

  // When a folder picked as source or target finishes scanning, select its index.
  useEffect(() => {
    if (!pendingScan) return;
    const job = jobs.find((j) => j.id === pendingScan.id);
    if (!job?.finishedAt) return;
    if (job.outcome) {
      const id = indexIdFromPath(job.outcome.index_path);
      (pendingScan.side === "source" ? setSource : setTarget)(id);
    } else if (job.error) {
      setError(job.error);
    }
    setPendingScan(null);
  }, [jobs, pendingScan]);

  const scanFolder = async (side: Side) => {
    const picked = await open({ directory: true, title: `Choose the ${side} folder` });
    if (typeof picked !== "string") return;
    try {
      setError(null);
      const id = await start(picked);
      setPendingScan({ side, id });
    } catch (e) {
      setError(e as CommandError);
    }
  };

  const run = async () => {
    setRunning(true);
    setError(null);
    try {
      setResult(await compareApi.compare(source, target, subpath || null));
    } catch (e) {
      setResult(null);
      setError(e as CommandError);
    } finally {
      setRunning(false);
    }
  };

  const scanning = pendingScan ? jobs.find((j) => j.id === pendingScan.id) : undefined;

  return (
    <>
      <PageHeader
        title="Compare"
        subtitle="Is everything from the source on the target? Matching is by content, regardless of folders and names."
      />

      <Panel className="mb-4">
        <div className="p-4 grid grid-cols-[1fr_1fr_220px_auto] gap-3 items-end">
          <IndexPicker
            label="Source"
            indexes={indexes}
            value={source}
            onChange={setSource}
            onFolder={() => scanFolder("source")}
          />
          <IndexPicker
            label="Target"
            indexes={indexes}
            value={target}
            onChange={setTarget}
            onFolder={() => scanFolder("target")}
          />
          <Field label="Source subfolder (optional)">
            <input
              value={subpath}
              onChange={(e) => setSubpath(e.target.value)}
              placeholder="e.g. 04_Partnere"
              className="h-8 w-full px-2.5 rounded-md border border-border bg-bg mono text-[12px] outline-none focus:border-cyan/60"
            />
          </Field>
          <Button
            variant="primary"
            className="h-8"
            disabled={!source || !target || running || !!pendingScan}
            onClick={run}
          >
            <Icon name="compare" /> {running ? "Comparing…" : "Compare"}
          </Button>
        </div>
        {scanning && (
          <div className="px-4 pb-3 text-[12px] text-muted">
            Scanning <span className="mono">{scanning.path}</span> ·{" "}
            <span className="mono tabular">{count(scanning.progress?.files_done ?? 0)}</span> /{" "}
            <span className="mono tabular">{count(scanning.progress?.files_seen ?? 0)}</span> files
          </div>
        )}
      </Panel>

      {error &&
        (error.kind === "target_unavailable" ? (
          <ConnectDrive message={error.message} indexes={indexes} target={target} />
        ) : (
          <div className="mb-4">
            <ErrorNote>{error.message}</ErrorNote>
          </div>
        ))}

      {result && <Results result={result} source={source} target={target} subpath={subpath || null} />}
    </>
  );
}

function ConnectDrive({
  message,
  indexes,
  target,
}: {
  message: string;
  indexes: IndexSummary[];
  target: string;
}) {
  const name =
    indexes.find((i) => i.id === target)?.volume_name ??
    message.match(/\/Volumes\/([^/]+)/)?.[1] ??
    "the drive";
  return (
    <Panel className="mb-4 border-amber/40">
      <div className="p-6 flex items-center gap-5">
        <div className="size-10 rounded-lg border border-amber/40 grid place-items-center text-amber">
          <Icon name="drive" />
        </div>
        <div>
          <div className="text-[16px] font-semibold">Connect {name}</div>
          <div className="text-[12px] text-muted mt-1 max-w-[720px]">
            The drive is not mounted or not readable, so DriveLedger cannot tell what is on it. Nothing
            is reported as missing. Connect the drive and try again.
          </div>
          <div className="mono text-[11px] text-faint mt-2">{message}</div>
        </div>
      </div>
    </Panel>
  );
}

function Results({
  result,
  source,
  target,
  subpath,
}: {
  result: CompareResult;
  source: string;
  target: string;
  subpath: string | null;
}) {
  const [statuses, setStatuses] = useState<Set<Status>>(
    () => new Set<Status>(STATUSES.filter((s) => s !== "present")),
  );
  const [ext, setExt] = useState("");
  const [query, setQuery] = useState("");
  const [folder, setFolder] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [detail, setDetail] = useState<FileComparison | null>(null);
  const [verified, setVerified] = useState<Map<string, VerifyItem>>(new Map());
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<{ tone: "cyan" | "magenta"; text: string } | null>(null);

  const extensions = useMemo(
    () => [...new Set(result.files.map((f) => extOf(f.rel_path)).filter(Boolean))].sort(),
    [result],
  );

  const files = useMemo(() => {
    const q = query.trim().toLowerCase();
    return result.files.filter(
      (f) =>
        statuses.has(f.status) &&
        (!ext || extOf(f.rel_path) === ext) &&
        (!q || f.rel_path.toLowerCase().includes(q)) &&
        (!folder || f.rel_path.startsWith(`${folder}/`)),
    );
  }, [result, statuses, ext, query, folder]);

  const toggleStatus = (s: Status) =>
    setStatuses((prev) => {
      const next = new Set(prev);
      if (next.has(s)) next.delete(s);
      else next.add(s);
      return next;
    });

  const verifySelected = async () => {
    setBusy("verify");
    setNote(null);
    try {
      const report = await compareApi.verify(source, target, [...selected]);
      setVerified((prev) => {
        const next = new Map(prev);
        report.items.forEach((i) => next.set(i.source_path, i));
        return next;
      });
      const s = report.summary;
      const bad = s.mismatch.files + s.unreadable.files;
      setNote({
        tone: bad ? "magenta" : "cyan",
        text: `${count(s.verified.files)} verified identical · ${count(s.mismatch.files)} mismatch · ${count(
          s.unreadable.files,
        )} unreadable · ${count(s.not_found_on_target.files)} had no match to verify`,
      });
    } catch (e) {
      setNote({ tone: "magenta", text: errorMessage(e) });
    } finally {
      setBusy(null);
    }
  };

  const exportReport = async () => {
    const out = await save({
      title: "Export comparison report",
      defaultPath: `${result.source.volume_name}-vs-${result.target.volume_name}.tsv`,
      filters: [
        { name: "TSV", extensions: ["tsv"] },
        { name: "JSON", extensions: ["json"] },
      ],
    });
    if (!out) return;
    setBusy("export");
    try {
      await compareApi.exportReport(source, target, subpath, out, out.endsWith(".json") ? "json" : "tsv");
      setNote({ tone: "cyan", text: `Report written to ${out}` });
    } catch (e) {
      setNote({ tone: "magenta", text: errorMessage(e) });
    } finally {
      setBusy(null);
    }
  };

  const s = result.summary;
  return (
    <div className="space-y-4">
      <div className="text-[12px] text-muted">
        <span className="text-text font-medium">{result.source.volume_name}</span>{" "}
        <span className="mono">{result.source.root_path}</span>
        {result.source_subpath && <span className="mono">/{result.source_subpath}</span>}
        <span className="mx-2 text-faint">→</span>
        <span className="text-text font-medium">{result.target.volume_name}</span>{" "}
        <span className="mono">{result.target.root_path}</span>
        <span className="text-faint"> · target scanned {date(result.target.scanned_at)}</span>
      </div>

      {result.warnings.map((w) => (
        <div key={w} className="rounded-md border border-amber/40 bg-amber/5 px-3 py-2 text-amber text-[12px]">
          {w}
        </div>
      ))}

      <div className="grid grid-cols-[1.3fr_repeat(5,1fr)] gap-3">
        <div
          className={`rounded-lg border p-4 ${
            s.content_not_found.files ? "border-magenta/50 bg-magenta/[0.06]" : "border-cyan/50 bg-cyan/[0.06]"
          }`}
        >
          <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted">Not backed up</div>
          <div className={`mt-2 mono tabular text-[26px] leading-none ${s.content_not_found.files ? "text-magenta" : "text-cyan"}`}>
            {bytes(s.content_not_found.bytes)}
          </div>
          <div className="mt-2 text-[12px] text-muted">
            <span className="mono tabular text-text">{count(s.content_not_found.files)}</span> of{" "}
            <span className="mono tabular">{count(s.total.files)}</span> files
          </div>
        </div>
        {STATUSES.map((status) => {
          const t = s.by_status[status];
          const active = statuses.has(status);
          return (
            <button
              key={status}
              onClick={() => toggleStatus(status)}
              className={`text-left rounded-lg border p-4 transition-colors ${
                active ? "border-border bg-surface" : "border-border/50 bg-surface/40 opacity-50"
              } hover:border-faint`}
            >
              <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-muted">
                <span className="size-1.5 rounded-full" style={{ background: `var(--status-${status.replace(/_/g, "-")})` }} />
                {STATUS_LABEL[status]}
              </div>
              <div className={`mt-2 mono tabular text-[22px] leading-none ${toneText[STATUS_TONE[status]]}`}>
                {count(t.files)}
              </div>
              <div className="mt-2 mono tabular text-[12px] text-muted">{bytes(t.bytes)}</div>
            </button>
          );
        })}
      </div>

      <FolderTable result={result} selected={folder} onSelect={setFolder} />

      <Panel
        title={`Files · ${count(files.length)}`}
        actions={
          <>
            <select
              value={ext}
              onChange={(e) => setExt(e.target.value)}
              className="h-7 px-2 rounded-md border border-border bg-surface-2 mono text-[11px] outline-none"
            >
              <option value="">all extensions</option>
              {extensions.map((x) => (
                <option key={x} value={x}>
                  .{x}
                </option>
              ))}
            </select>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter path…"
              className="h-7 w-56 px-2.5 rounded-md border border-border bg-bg mono text-[11px] outline-none focus:border-cyan/60"
            />
            <Button disabled={selected.size === 0 || busy !== null} onClick={verifySelected}>
              {busy === "verify" ? "Verifying…" : `Verify selected (${count(selected.size)})`}
            </Button>
            <Button disabled={busy !== null} onClick={exportReport}>
              Export report…
            </Button>
          </>
        }
      >
        {folder && (
          <div className="px-4 py-2 border-b border-border text-[12px] text-muted flex items-center gap-2">
            Folder <span className="mono text-text">{folder}</span>
            <button className="text-faint hover:text-text" onClick={() => setFolder(null)}>
              <Icon name="x" className="size-3" />
            </button>
          </div>
        )}
        {note && (
          <div className={`px-4 py-2 border-b border-border text-[12px] ${toneText[note.tone]}`}>{note.text}</div>
        )}
        <FileTable
          files={files}
          selected={selected}
          setSelected={setSelected}
          verified={verified}
          active={detail}
          onOpen={setDetail}
        />
      </Panel>

      {detail && (
        <Detail file={detail} result={result} verified={verified.get(detail.rel_path)} onClose={() => setDetail(null)} />
      )}
    </div>
  );
}

function FolderTable({
  result,
  selected,
  onSelect,
}: {
  result: CompareResult;
  selected: string | null;
  onSelect: (folder: string | null) => void;
}) {
  const [sort, setSort] = useState<"folder" | "size" | "missing">("missing");
  const rows = useMemo(() => {
    const notFound = (f: CompareResult["folders"][number]) =>
      f.by_status.missing.bytes + f.by_status.same_name_different_content.bytes;
    const sorted = [...result.folders];
    if (sort === "size") sorted.sort((a, b) => b.total.bytes - a.total.bytes);
    else if (sort === "missing") sorted.sort((a, b) => notFound(b) - notFound(a) || a.folder.localeCompare(b.folder));
    else sorted.sort((a, b) => a.folder.localeCompare(b.folder));
    return sorted;
  }, [result, sort]);

  const head = (key: typeof sort, label: string, right = true) => (
    <th
      onClick={() => setSort(key)}
      className={`h-8 px-3 font-semibold cursor-pointer hover:text-text ${right ? "text-right" : "text-left"} ${
        sort === key ? "text-cyan" : ""
      }`}
    >
      {label}
    </th>
  );
  const cell = (n: number, status: Status) => (
    <td className={`px-3 text-right mono tabular ${n ? toneText[STATUS_TONE[status]] : "text-faint"}`}>
      {n ? count(n) : "·"}
    </td>
  );

  return (
    <Panel title={`Folders · ${count(result.folders.length)}`}>
      <div className="max-h-[300px] overflow-auto">
        <table className="w-full text-[12px]">
          <thead className="sticky top-0 bg-surface">
            <tr className="text-[10px] uppercase tracking-[0.12em] text-faint">
              {head("folder", "Folder", false)}
              <th className="h-8 px-3 text-right font-semibold">Files</th>
              {head("size", "Size")}
              <th className="h-8 px-3 text-right font-semibold">Present</th>
              <th className="h-8 px-3 text-right font-semibold">Elsewhere</th>
              <th className="h-8 px-3 text-right font-semibold">Bundle</th>
              <th className="h-8 px-3 text-right font-semibold">Different</th>
              <th className="h-8 px-3 text-right font-semibold">Missing</th>
              {head("missing", "Not found")}
            </tr>
          </thead>
          <tbody>
            {rows.map((f) => {
              const nf = f.by_status.missing.bytes + f.by_status.same_name_different_content.bytes;
              return (
                <tr
                  key={f.folder}
                  onClick={() => onSelect(selected === f.folder ? null : f.folder)}
                  className={`h-7 border-t border-border/60 cursor-pointer ${
                    selected === f.folder ? "bg-cyan/10" : "hover:bg-surface-2/60"
                  }`}
                >
                  <td className="px-3 mono truncate max-w-[520px]" title={f.folder}>
                    {f.folder}
                  </td>
                  <td className="px-3 text-right mono tabular">{count(f.total.files)}</td>
                  <td className="px-3 text-right mono tabular">{bytes(f.total.bytes)}</td>
                  {cell(f.by_status.present.files, "present")}
                  {cell(f.by_status.present_elsewhere.files, "present_elsewhere")}
                  {cell(f.by_status.only_in_bundle.files, "only_in_bundle")}
                  {cell(f.by_status.same_name_different_content.files, "same_name_different_content")}
                  {cell(f.by_status.missing.files, "missing")}
                  <td className={`px-3 text-right mono tabular ${nf ? "text-magenta" : "text-faint"}`}>
                    {nf ? bytes(nf) : "·"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}

const ROW = 30;

function FileTable({
  files,
  selected,
  setSelected,
  verified,
  active,
  onOpen,
}: {
  files: FileComparison[];
  selected: Set<string>;
  setSelected: (s: Set<string>) => void;
  verified: Map<string, VerifyItem>;
  active: FileComparison | null;
  onOpen: (f: FileComparison) => void;
}) {
  const parent = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: files.length,
    getScrollElement: () => parent.current,
    estimateSize: () => ROW,
    overscan: 20,
  });

  if (files.length === 0) return <Empty>No files match the filters.</Empty>;

  const allSelected = files.length > 0 && files.every((f) => selected.has(f.rel_path));
  const toggleAll = () => setSelected(allSelected ? new Set() : new Set(files.map((f) => f.rel_path)));
  const toggle = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const cols = "grid grid-cols-[32px_150px_90px_90px_minmax(0,1.3fr)_minmax(0,1fr)] items-center";
  return (
    <div>
      <div className={`${cols} h-8 border-b border-border text-[10px] uppercase tracking-[0.12em] text-faint font-semibold`}>
        <div className="pl-4">
          <input type="checkbox" checked={allSelected} onChange={toggleAll} className="accent-cyan" />
        </div>
        <div className="px-2">Status</div>
        <div className="px-2 text-right">Size</div>
        <div className="px-2">Modified</div>
        <div className="px-2">Path</div>
        <div className="px-2 pr-4">On target</div>
      </div>
      <div ref={parent} className="h-[460px] overflow-auto">
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((row) => {
            const f = files[row.index];
            const v = verified.get(f.rel_path);
            const first = f.target_locations[0];
            return (
              <div
                key={f.rel_path}
                onClick={() => onOpen(f)}
                style={{ position: "absolute", top: 0, left: 0, right: 0, height: ROW, transform: `translateY(${row.start}px)` }}
                className={`${cols} text-[12px] border-b border-border/40 cursor-pointer ${
                  active?.rel_path === f.rel_path ? "bg-surface-2" : "hover:bg-surface-2/50"
                }`}
              >
                <div className="pl-4" onClick={(e) => e.stopPropagation()}>
                  <input
                    type="checkbox"
                    checked={selected.has(f.rel_path)}
                    onChange={() => toggle(f.rel_path)}
                    className="accent-cyan"
                  />
                </div>
                <div className="px-2 flex items-center gap-1 min-w-0">
                  <Badge tone={STATUS_TONE[f.status]}>{shortStatus[f.status]}</Badge>
                  {v && <Badge tone={v.status === "verified" ? "cyan" : "magenta"}>{v.status === "verified" ? "✓" : "!"}</Badge>}
                </div>
                <div className="px-2 text-right mono tabular">{bytes(f.size)}</div>
                <div className="px-2 mono tabular text-muted">{date(f.modified)}</div>
                <div className="px-2 mono truncate" title={f.rel_path}>
                  {f.rel_path}
                </div>
                <div className="px-2 pr-4 mono truncate text-muted" title={first?.rel_path}>
                  {first ? (
                    <>
                      <span className="text-faint">{f.status === "same_name_different_content" ? "≠ " : "→ "}</span>
                      {first.rel_path}
                      {f.target_locations.length > 1 && (
                        <span className="text-faint"> +{f.target_locations.length - 1}</span>
                      )}
                    </>
                  ) : (
                    <span className="text-faint">–</span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

const shortStatus: Record<Status, string> = {
  present: "present",
  present_elsewhere: "elsewhere",
  only_in_bundle: "bundle",
  same_name_different_content: "different",
  missing: "missing",
};

function Detail({
  file,
  result,
  verified,
  onClose,
}: {
  file: FileComparison;
  result: CompareResult;
  verified?: VerifyItem;
  onClose: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const reveal = async (root: string, rel: string) => {
    setError(null);
    try {
      if (!(await compareApi.isAvailable(root))) {
        setError(`${root} is not mounted. Connect the drive to reveal the file.`);
        return;
      }
      await revealItemInDir(joinPath(root, rel));
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  const hints = [
    ...file.hints.map((h) => (h === "possibly_truncated" ? "Possibly truncated (interrupted copy)" : "Source was unreadable during the scan")),
    ...(file.newer_side === "target" ? ["Target is newer"] : file.newer_side === "source" ? ["Source is newer"] : []),
  ];

  return (
    <Panel
      title="File details"
      actions={
        <Button onClick={onClose}>
          <Icon name="x" /> Close
        </Button>
      }
    >
      <div className="p-4 space-y-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Badge tone={STATUS_TONE[file.status]}>{STATUS_LABEL[file.status]}</Badge>
              {hints.map((h) => (
                <Badge key={h} tone="amber">
                  {h}
                </Badge>
              ))}
            </div>
            <div className="mono text-[13px] mt-2 break-all">{file.rel_path}</div>
            <div className="mono tabular text-[12px] text-muted mt-1">
              {bytes(file.size)} · {file.size.toLocaleString("en-US")} bytes · modified {date(file.modified)}
            </div>
          </div>
          <Button onClick={() => reveal(result.source.root_path, file.rel_path)}>Reveal source in Finder</Button>
        </div>

        {verified && (
          <div className={`text-[12px] ${verified.status === "verified" ? "text-cyan" : "text-magenta"}`}>
            Full-file verify: {verified.status}
            {verified.message && ` · ${verified.message}`}
            {verified.source_hash && <div className="mono text-[11px] text-faint mt-1">BLAKE3 {verified.source_hash}</div>}
          </div>
        )}

        <div>
          <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-faint mb-2">
            {file.status === "same_name_different_content"
              ? "Same name on target (different content)"
              : file.status === "missing"
                ? "Not found on target"
                : `Identical content on target · ${file.target_locations.length}`}
          </div>
          {file.target_locations.map((l) => (
            <div key={l.rel_path} className="flex items-center gap-3 h-8 border-t border-border/60 text-[12px]">
              <span className="mono truncate flex-1" title={l.rel_path}>
                {l.rel_path}
              </span>
              {l.in_bundle && <Badge tone="violet">bundle</Badge>}
              <span className="mono tabular text-muted w-24 text-right">{bytes(l.size)}</span>
              <span className="mono tabular text-muted w-24">{date(l.modified)}</span>
              <Button onClick={() => reveal(result.target.root_path, l.rel_path)}>Reveal</Button>
            </div>
          ))}
        </div>
        {error && <ErrorNote>{error}</ErrorNote>}
      </div>
    </Panel>
  );
}

function IndexPicker({
  label,
  indexes,
  value,
  onChange,
  onFolder,
}: {
  label: string;
  indexes: IndexSummary[];
  value: string;
  onChange: (id: string) => void;
  onFolder: () => void;
}) {
  return (
    <Field label={label}>
      <div className="flex gap-1.5">
        <select
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="h-8 flex-1 min-w-0 px-2 rounded-md border border-border bg-bg text-[12px] outline-none focus:border-cyan/60"
        >
          <option value="">Choose a saved index…</option>
          {indexes.map((i) => (
            <option key={i.id} value={i.id}>
              {i.volume_name} · {i.root_path} · {date(i.scanned_at)} · {count(i.file_count)} files
            </option>
          ))}
        </select>
        <Button className="h-8" onClick={onFolder} title="Scan a folder and use it">
          <Icon name="folder" /> Folder…
        </Button>
      </div>
    </Field>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block min-w-0">
      <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-faint mb-1.5">{label}</div>
      {children}
    </label>
  );
}

function extOf(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}
