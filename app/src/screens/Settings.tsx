import { useEffect, useState, type KeyboardEvent } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type Settings as SettingsData, type SettingsView } from "../lib/api";
import { Button, ErrorNote, Icon, PageHeader, Panel } from "../components/ui";

export default function Settings() {
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<SettingsData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    api
      .getSettings()
      .then((v) => {
        setView(v);
        setDraft(v.settings);
      })
      .catch((e) => setError(errorMessage(e)));
  }, []);

  if (!view || !draft) return error ? <ErrorNote>{error}</ErrorNote> : null;

  const update = (patch: Partial<SettingsData>) => {
    setDraft({ ...draft, ...patch });
    setSaved(false);
  };

  const pickLibrary = async () => {
    const picked = await open({
      directory: true,
      defaultPath: draft.library ?? view.default_library,
      title: "Choose the DriveLedger library folder",
    });
    if (typeof picked === "string") update({ library: picked });
  };

  const save = async () => {
    try {
      const next = await api.saveSettings(draft);
      setView(next);
      setDraft(next.settings);
      setSaved(true);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const dirty = JSON.stringify(draft) !== JSON.stringify(view.settings);

  return (
    <>
      <PageHeader
        title="Settings"
        subtitle={
          view.settings_path && (
            <>
              Saved in <span className="mono">{view.settings_path}</span>
            </>
          )
        }
        actions={
          <>
            {saved && !dirty && <span className="text-[12px] text-cyan">Saved</span>}
            <Button variant="primary" disabled={!dirty} onClick={save}>
              Save
            </Button>
          </>
        }
      />
      {error && <div className="mb-4"><ErrorNote>{error}</ErrorNote></div>}

      <div className="space-y-4">
        <Panel title="Library folder">
          <div className="p-4 space-y-3">
            <p className="text-[12px] text-muted max-w-[720px]">
              Index files are written here, and nowhere else. DriveLedger refuses a library inside a
              folder it is scanning.
            </p>
            <div className="flex items-center gap-2">
              <div className="flex-1 h-8 px-3 flex items-center rounded-md border border-border bg-bg mono text-[12px] truncate">
                {draft.library ?? `${view.default_library} (default)`}
              </div>
              <Button onClick={pickLibrary}>
                <Icon name="folder" /> Choose…
              </Button>
              <Button disabled={draft.library === null} onClick={() => update({ library: null })}>
                Use default
              </Button>
            </div>
          </div>
        </Panel>

        <Panel title="Skip list">
          <div className="p-4 space-y-4">
            <p className="text-[12px] text-muted max-w-[720px]">
              Names that are never indexed. Matching folders are skipped with everything inside them.
            </p>
            <TokenList
              label="Exact names"
              values={draft.skip.names}
              onChange={(names) => update({ skip: { ...draft.skip, names } })}
            />
            <TokenList
              label="Name prefixes"
              values={draft.skip.prefixes}
              onChange={(prefixes) => update({ skip: { ...draft.skip, prefixes } })}
            />
          </div>
        </Panel>

        <Panel title="Bundles">
          <div className="p-4 space-y-4">
            <p className="text-[12px] text-muted max-w-[720px]">
              Folder extensions treated as packages. Files inside them are indexed but flagged, and a
              copy that only exists inside a bundle is reported as “only in bundle”.
            </p>
            <TokenList
              label="Extensions"
              values={draft.bundle_extensions}
              onChange={(bundle_extensions) => update({ bundle_extensions })}
            />
          </div>
        </Panel>
      </div>
    </>
  );
}

/** Editable list of short strings. Shows control characters visibly. */
function TokenList({
  label,
  values,
  onChange,
}: {
  label: string;
  values: string[];
  onChange: (values: string[]) => void;
}) {
  const [input, setInput] = useState("");
  const add = () => {
    const value = input.trim();
    if (value && !values.includes(value)) onChange([...values, value]);
    setInput("");
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") add();
  };
  return (
    <div>
      <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-faint mb-2">{label}</div>
      <div className="flex flex-wrap gap-1.5 items-center">
        {values.map((v) => (
          <span
            key={v}
            className="inline-flex items-center gap-1 h-6 pl-2 pr-1 rounded border border-border bg-surface-2 mono text-[11px]"
          >
            {v.replace("\r", "\\r")}
            <button
              className="text-faint hover:text-magenta"
              onClick={() => onChange(values.filter((x) => x !== v))}
              aria-label={`Remove ${v}`}
            >
              <Icon name="x" className="size-3" />
            </button>
          </span>
        ))}
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKey}
          onBlur={add}
          placeholder="Add…"
          className="h-6 w-28 px-2 rounded border border-dashed border-border bg-transparent mono text-[11px] outline-none focus:border-cyan/60"
        />
      </div>
    </div>
  );
}
