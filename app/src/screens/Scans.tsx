import { useEffect, useState } from "react";
import { indexIdFromPath } from "../lib/api";
import { bytes, count, duration } from "../lib/format";
import { useScans, type ScanJob } from "../lib/scans";
import { Badge, Button, Empty, ErrorNote, Icon, PageHeader, Panel, Progress, Stat } from "../components/ui";

export default function Scans({ onDone }: { onDone: () => void }) {
  const { jobs } = useScans();
  return (
    <>
      <PageHeader
        title="Scans"
        subtitle="Scans read file metadata and three 1 MiB samples per file. Nothing on the drive is changed."
        actions={<Button onClick={onDone}>Back to drives</Button>}
      />
      {jobs.length === 0 ? (
        <Panel>
          <Empty>No scans in this session. Start one from Drives.</Empty>
        </Panel>
      ) : (
        <div className="space-y-3">
          {jobs.map((job) => (
            <ScanCard key={job.id} job={job} />
          ))}
        </div>
      )}
    </>
  );
}

function ScanCard({ job }: { job: ScanJob }) {
  const { cancel, dismiss } = useScans();
  const now = useNow(!job.finishedAt);
  const p = job.progress;
  const elapsed = (job.finishedAt ?? now) - job.startedAt;
  const fraction =
    p && p.phase === "processing" && p.files_seen > 0 ? p.files_done / p.files_seen : undefined;

  const state = job.error
    ? job.error.kind === "cancelled"
      ? { label: "cancelled", tone: "muted" as const }
      : { label: "failed", tone: "magenta" as const }
    : job.outcome
      ? { label: "done", tone: "cyan" as const }
      : { label: p?.phase ?? "starting", tone: "violet" as const };

  return (
    <Panel>
      <div className="p-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <Badge tone={state.tone}>{state.label}</Badge>
              <span className="mono text-[13px] truncate" title={job.path}>
                {job.path}
              </span>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            <span className="mono tabular text-[12px] text-muted">{duration(elapsed)}</span>
            {!job.finishedAt ? (
              <Button variant="danger" disabled={job.cancelling} onClick={() => cancel(job.id)}>
                <Icon name="x" /> {job.cancelling ? "Cancelling…" : "Cancel"}
              </Button>
            ) : (
              <Button onClick={() => dismiss(job.id)}>Dismiss</Button>
            )}
          </div>
        </div>

        {!job.finishedAt && (
          <>
            <div className="mt-4">
              <Progress value={fraction} />
            </div>
            <div className="mt-4 grid grid-cols-4 gap-6">
              <Stat label="Files found" value={count(p?.files_seen ?? 0)} />
              <Stat label="Processed" value={count(p?.files_done ?? 0)} />
              <Stat label="Bytes hashed" value={bytes(p?.bytes_hashed ?? 0)} />
              <Stat label="Errors" value={count(p?.errors ?? 0)} tone={p?.errors ? "amber" : undefined} />
            </div>
            <div className="mt-4 mono text-[11px] text-faint truncate" title={p?.current_path}>
              {p?.current_path || "…"}
            </div>
          </>
        )}

        {job.outcome && (
          <div className="mt-4 grid grid-cols-5 gap-6">
            <Stat label="Files" value={count(job.outcome.info.file_count)} />
            <Stat label="Size" value={bytes(job.outcome.info.total_bytes)} />
            <Stat label="Hashed" value={count(job.outcome.stats.files_hashed)} />
            <Stat label="Reused" value={count(job.outcome.stats.files_reused)} />
            <Stat
              label="Errors"
              value={count(job.outcome.info.error_count)}
              tone={job.outcome.info.error_count ? "amber" : undefined}
            />
            <div className="col-span-5 text-[11px] text-muted">
              Index <span className="mono text-text/80">{indexIdFromPath(job.outcome.index_path)}</span>
            </div>
          </div>
        )}

        {job.error && job.error.kind !== "cancelled" && (
          <div className="mt-4">
            <ErrorNote>{job.error.message}</ErrorNote>
          </div>
        )}
      </div>
    </Panel>
  );
}

/** Current time, ticking every second while `active`. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!active) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [active]);
  return now;
}
