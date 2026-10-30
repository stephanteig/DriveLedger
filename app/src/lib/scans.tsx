// App-wide state for running and finished scans, fed by Tauri events.

import { listen } from "@tauri-apps/api/event";
import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import {
  api,
  type CommandError,
  type ScanFinishedEvent,
  type ScanOutcome,
  type ScanProgress,
  type ScanProgressEvent,
} from "./api";

export interface ScanJob {
  id: string;
  path: string;
  startedAt: number;
  finishedAt?: number;
  progress?: ScanProgress;
  outcome?: ScanOutcome;
  error?: CommandError;
  cancelling?: boolean;
}

interface ScansContextValue {
  jobs: ScanJob[];
  start: (path: string, full?: boolean) => Promise<string>;
  cancel: (id: string) => Promise<void>;
  dismiss: (id: string) => void;
  /** Bumped whenever a scan finishes, so lists can refresh. */
  generation: number;
}

const ScansContext = createContext<ScansContextValue | null>(null);

export function ScansProvider({ children }: { children: ReactNode }) {
  const [jobs, setJobs] = useState<ScanJob[]>([]);
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    const update = (id: string, patch: Partial<ScanJob>) =>
      setJobs((all) => all.map((j) => (j.id === id ? { ...j, ...patch } : j)));
    const unlisten = [
      listen<ScanProgressEvent>("scan://progress", (e) =>
        update(e.payload.scan_id, { progress: e.payload.progress }),
      ),
      listen<ScanFinishedEvent>("scan://finished", (e) => {
        update(e.payload.scan_id, {
          outcome: e.payload.outcome ?? undefined,
          error: e.payload.error ?? undefined,
          finishedAt: Date.now(),
        });
        setGeneration((g) => g + 1);
      }),
    ];
    return () => {
      unlisten.forEach((p) => p.then((off) => off()));
    };
  }, []);

  const start = useCallback(async (path: string, full = false) => {
    const id = await api.startScan(path, full);
    setJobs((all) => [{ id, path, startedAt: Date.now() }, ...all]);
    return id;
  }, []);

  const cancel = useCallback(async (id: string) => {
    setJobs((all) => all.map((j) => (j.id === id ? { ...j, cancelling: true } : j)));
    await api.cancelScan(id);
  }, []);

  const dismiss = useCallback((id: string) => {
    setJobs((all) => all.filter((j) => j.id !== id));
  }, []);

  return (
    <ScansContext.Provider value={{ jobs, start, cancel, dismiss, generation }}>
      {children}
    </ScansContext.Provider>
  );
}

export function useScans(): ScansContextValue {
  const value = useContext(ScansContext);
  if (!value) throw new Error("useScans outside ScansProvider");
  return value;
}
