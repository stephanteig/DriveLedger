import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Icon } from "./components/ui";
import { ScansProvider, useScans } from "./lib/scans";
import Drives from "./screens/Drives";
import Scans from "./screens/Scans";
import Settings from "./screens/Settings";
import Compare from "./screens/Compare";
import Search from "./screens/Search";

export type Screen = "drives" | "scans" | "compare" | "search" | "settings";

const nav: { id: Screen; label: string; icon: Parameters<typeof Icon>[0]["name"] }[] = [
  { id: "drives", label: "Drives", icon: "drive" },
  { id: "scans", label: "Scans", icon: "scan" },
  { id: "compare", label: "Compare", icon: "compare" },
  { id: "search", label: "Search", icon: "search" },
  { id: "settings", label: "Settings", icon: "settings" },
];

export default function App() {
  return (
    <ScansProvider>
      <Shell />
    </ScansProvider>
  );
}

function Shell() {
  const [screen, setScreen] = useState<Screen>("drives");
  const [version, setVersion] = useState("");
  const { jobs } = useScans();
  const running = jobs.filter((j) => !j.finishedAt).length;

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  return (
    <div className="flex h-full">
      <aside className="w-[208px] shrink-0 flex flex-col border-r border-border bg-surface">
        <div className="h-14 flex items-center gap-2.5 px-4 border-b border-border">
          <Logo />
          <span className="text-[15px] font-semibold tracking-tight">DriveLedger</span>
        </div>
        <nav className="flex-1 p-2 space-y-0.5">
          {nav.map((item) => {
            const active = screen === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setScreen(item.id)}
                className={`w-full flex items-center gap-2.5 h-8 px-2.5 rounded-md text-[13px] transition-colors ${
                  active
                    ? "bg-surface-2 text-text shadow-[inset_2px_0_0_var(--color-cyan)]"
                    : "text-muted hover:text-text hover:bg-surface-2/60"
                }`}
              >
                <Icon name={item.icon} className={active ? "text-cyan" : ""} />
                <span className="flex-1 text-left">{item.label}</span>
                {item.id === "scans" && running > 0 && (
                  <span className="mono text-[10px] text-cyan">{running}</span>
                )}
              </button>
            );
          })}
        </nav>
        <div className="p-4 border-t border-border text-[11px] leading-relaxed text-faint">
          <div className="flex items-center gap-1.5 text-muted">
            <span className="size-1.5 rounded-full bg-cyan" />
            Read-only on scanned drives
          </div>
          <div className="mono mt-1">v{version}</div>
        </div>
      </aside>

      <main className="flex-1 min-w-0 overflow-auto grid-bg">
        <div className="max-w-[1400px] mx-auto px-8 py-7">
          {screen === "drives" && <Drives onScanStarted={() => setScreen("scans")} />}
          {screen === "scans" && <Scans onDone={() => setScreen("drives")} />}
          {screen === "compare" && <Compare />}
          {screen === "search" && <Search />}
          {screen === "settings" && <Settings />}
        </div>
      </main>
    </div>
  );
}

function Logo() {
  // Two offset ledger columns: source and target.
  return (
    <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden>
      <rect x="2" y="3" width="7" height="14" rx="1.5" fill="none" stroke="var(--color-cyan)" strokeWidth="1.5" />
      <rect x="11" y="3" width="7" height="14" rx="1.5" fill="none" stroke="var(--color-violet)" strokeWidth="1.5" />
      <path d="M4.5 7h2M4.5 10h2M4.5 13h2M13.5 7h2M13.5 10h2" stroke="var(--color-cyan)" strokeWidth="1.5" strokeLinecap="round" />
      <path d="M13.5 13h2" stroke="var(--color-magenta)" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}
