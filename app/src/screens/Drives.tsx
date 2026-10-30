import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type IndexSummary, type VolumeStatus } from "../lib/api";
import { bytes, count, date } from "../lib/format";
import { useScans } from "../lib/scans";
import { Badge, Button, Empty, ErrorNote, Icon, PageHeader, Panel } from "../components/ui";

export default function Drives({ onScanStarted }: { onScanStarted: () => void }) {
  const { start, generation } = useScans();
  const [volumes, setVolumes] = useState<VolumeStatus[] | null>(null);
  const [indexes, setIndexes] = useState<IndexSummary[] | null>(null);
  const [library, setLibrary] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [v, i, s] = await Promise.all([api.listVolumes(), api.listIndexes(), api.getSettings()]);
      setVolumes(v);
      setIndexes(i);
      setLibrary(s.library);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, generation]);

  const scan = async (path: string) => {
    try {
      await start(path);
      onScanStarted();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const scanFolder = async (defaultPath?: string) => {
    const picked = await open({ directory: true, defaultPath, title: "Choose a folder to scan" });
    if (typeof picked === "string") await scan(picked);
  };

  return (
    <>
      <PageHeader
        title="Drives"
        subtitle={
          <>
            Library <span className="mono text-text/80">{library}</span>
          </>
        }
        actions={
          <>
            <Button onClick={() => scanFolder()}>
              <Icon name="folder" /> Scan folder…
            </Button>
            <Button onClick={refresh}>
              <Icon name="refresh" /> Refresh
            </Button>
          </>
        }
      />

      {error && <div className="mb-4"><ErrorNote>{error}</ErrorNote></div>}

      <div className="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-3 mb-6">
        {volumes?.map((v) => (
          <VolumeCard
            key={v.volume.mount_point}
            status={v}
            onScan={() => scan(v.volume.mount_point)}
            onScanFolder={() => scanFolder(v.volume.mount_point)}
          />
        ))}
        {volumes?.length === 0 && (
          <Panel className="col-span-full">
            <Empty>No mounted volumes found.</Empty>
          </Panel>
        )}
      </div>

      <Panel title={`Saved indexes${indexes ? ` · ${indexes.length}` : ""}`}>
        {indexes && indexes.length > 0 ? <IndexTable indexes={indexes} /> : (
          <Empty>No scans yet. Scan a drive to create its first index.</Empty>
        )}
      </Panel>
    </>
  );
}

function VolumeCard({
  status,
  onScan,
  onScanFolder,
}: {
  status: VolumeStatus;
  onScan: () => void;
  onScanFolder: () => void;
}) {
  const { volume, index_count, last_scanned_at } = status;
  const network = ["smbfs", "nfs", "afpfs", "webdav"].includes(volume.fs_type ?? "");
  return (
    <div className="group rounded-lg border border-border bg-surface hover:border-faint transition-colors">
      <div className="p-4">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="text-[15px] font-semibold truncate">{volume.name}</div>
            <div className="mono text-[11px] text-muted truncate mt-0.5">{volume.mount_point}</div>
          </div>
          <div className="flex gap-1 shrink-0">
            {volume.fs_type && <Badge tone={network ? "violet" : "muted"}>{volume.fs_type}</Badge>}
          </div>
        </div>
        <div className="mt-4 flex items-end justify-between">
          <div className="text-[11px] text-muted leading-relaxed">
            <div>
              <span className="mono text-text tabular">{index_count}</span> index{index_count === 1 ? "" : "es"}
            </div>
            <div>
              Last scan{" "}
              <span className={`mono tabular ${last_scanned_at ? "text-text" : "text-faint"}`}>
                {date(last_scanned_at)}
              </span>
            </div>
          </div>
          <div className="flex gap-1.5">
            <Button onClick={onScanFolder} title="Scan a subfolder">
              <Icon name="folder" />
            </Button>
            <Button variant="primary" onClick={onScan}>
              <Icon name="scan" /> Scan
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}

function IndexTable({ indexes }: { indexes: IndexSummary[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-[12px]">
        <thead>
          <tr className="text-left text-[10px] uppercase tracking-[0.12em] text-faint">
            <Th>Scanned</Th>
            <Th>Volume</Th>
            <Th>Root</Th>
            <Th right>Files</Th>
            <Th right>Size</Th>
            <Th right>Errors</Th>
            <Th>ID</Th>
          </tr>
        </thead>
        <tbody>
          {indexes.map((i) => (
            <tr key={i.id} className="border-t border-border/70 hover:bg-surface-2/60">
              <Td mono>{date(i.scanned_at)}</Td>
              <Td className="font-medium">{i.volume_name}</Td>
              <Td mono className="text-muted max-w-[420px] truncate" title={i.root_path}>
                {i.root_path}
              </Td>
              <Td mono right>{count(i.file_count)}</Td>
              <Td mono right>{bytes(i.total_bytes)}</Td>
              <Td mono right className={i.error_count ? "text-amber" : "text-faint"}>
                {i.error_count ? count(i.error_count) : "·"}
              </Td>
              <Td mono className="text-faint max-w-[260px] truncate" title={i.id}>
                {i.id}
              </Td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function Th({ children, right }: { children?: React.ReactNode; right?: boolean }) {
  return <th className={`h-8 px-4 font-semibold ${right ? "text-right" : ""}`}>{children}</th>;
}

export function Td({
  children,
  right,
  mono,
  className = "",
  title,
}: {
  children: React.ReactNode;
  right?: boolean;
  mono?: boolean;
  className?: string;
  title?: string;
}) {
  return (
    <td
      title={title}
      className={`h-8 px-4 whitespace-nowrap ${right ? "text-right" : ""} ${mono ? "mono tabular" : ""} ${className}`}
    >
      {children}
    </td>
  );
}
