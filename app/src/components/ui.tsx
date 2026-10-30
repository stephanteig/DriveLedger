import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "primary" | "ghost" | "danger";

const variants: Record<Variant, string> = {
  primary:
    "bg-cyan/10 text-cyan border-cyan/40 hover:bg-cyan/20 hover:border-cyan/70",
  ghost: "bg-surface-2 text-text border-border hover:border-faint",
  danger: "bg-magenta/10 text-magenta border-magenta/40 hover:bg-magenta/20",
};

export function Button({
  variant = "ghost",
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      {...props}
      className={`inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md border text-[12px] font-medium tracking-wide transition-colors disabled:opacity-40 disabled:pointer-events-none ${variants[variant]} ${className}`}
    />
  );
}

export function Panel({
  title,
  actions,
  children,
  className = "",
}: {
  title?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`rounded-lg border border-border bg-surface ${className}`}>
      {(title || actions) && (
        <header className="flex items-center justify-between h-10 px-4 border-b border-border">
          <h2 className="text-[11px] font-semibold uppercase tracking-[0.12em] text-muted">
            {title}
          </h2>
          <div className="flex items-center gap-2">{actions}</div>
        </header>
      )}
      {children}
    </section>
  );
}

export function PageHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="flex items-end justify-between gap-6 mb-5">
      <div className="min-w-0">
        <h1 className="text-[22px] font-semibold tracking-tight">{title}</h1>
        {subtitle && <div className="mt-1 text-muted text-[12px] truncate">{subtitle}</div>}
      </div>
      <div className="flex items-center gap-2 shrink-0">{actions}</div>
    </div>
  );
}

export function Badge({ children, tone = "muted" }: { children: ReactNode; tone?: Tone }) {
  return (
    <span
      className={`inline-flex items-center h-5 px-1.5 rounded border text-[10px] font-semibold uppercase tracking-wider ${tones[tone]}`}
    >
      {children}
    </span>
  );
}

export type Tone = "muted" | "cyan" | "violet" | "amber" | "magenta";

const tones: Record<Tone, string> = {
  muted: "border-border text-muted",
  cyan: "border-cyan/40 text-cyan",
  violet: "border-violet/40 text-violet",
  amber: "border-amber/40 text-amber",
  magenta: "border-magenta/40 text-magenta",
};

export function Empty({ children }: { children: ReactNode }) {
  return <div className="px-4 py-10 text-center text-muted text-[12px]">{children}</div>;
}

export function ErrorNote({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-md border border-magenta/40 bg-magenta/5 px-3 py-2 text-magenta text-[12px]">
      {children}
    </div>
  );
}

/** Thin progress bar; `value` in 0..1, or undefined for indeterminate. */
export function Progress({ value }: { value?: number }) {
  return (
    <div className="h-1 w-full rounded-full bg-surface-2 overflow-hidden">
      {value === undefined ? (
        <div className="h-full w-1/3 bg-cyan/70 animate-pulse" />
      ) : (
        <div
          className="h-full bg-cyan transition-[width] duration-150"
          style={{ width: `${Math.min(100, Math.max(0, value * 100))}%` }}
        />
      )}
    </div>
  );
}

/** Minimal stroke icons, 16px. */
export function Icon({ name, className = "" }: { name: IconName; className?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.4"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden
    >
      {icons[name]}
    </svg>
  );
}

type IconName = "drive" | "scan" | "compare" | "search" | "settings" | "refresh" | "folder" | "x";

const icons: Record<IconName, ReactNode> = {
  drive: (
    <>
      <rect x="2" y="4" width="12" height="8" rx="1.5" />
      <path d="M4.5 9.5h.01M7 9.5h4.5" />
    </>
  ),
  scan: <path d="M2 5V3a1 1 0 0 1 1-1h2M11 2h2a1 1 0 0 1 1 1v2M14 11v2a1 1 0 0 1-1 1h-2M5 14H3a1 1 0 0 1-1-1v-2M4.5 8h7" />,
  compare: <path d="M5 2v12M11 2v12M2 5h3M11 5h3M2 11h3M11 11h3" />,
  search: (
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="m10.5 10.5 3.5 3.5" />
    </>
  ),
  settings: (
    <>
      <circle cx="8" cy="8" r="2" />
      <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" />
    </>
  ),
  refresh: <path d="M13.5 8A5.5 5.5 0 1 1 11.9 4.1M13.5 2.5v3h-3" />,
  folder: <path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h2.6l1.4 1.5h5A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z" />,
  x: <path d="m4 4 8 8M12 4l-8 8" />,
};

export const toneText: Record<Tone, string> = {
  muted: "text-muted",
  cyan: "text-cyan",
  violet: "text-violet",
  amber: "text-amber",
  magenta: "text-magenta",
};

/** Label + value pair used in stat rows. */
export function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: ReactNode;
  tone?: Tone;
}) {
  const color = tone ? toneText[tone] : "text-text";
  return (
    <div className="min-w-0">
      <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-faint">{label}</div>
      <div className={`mt-1 mono tabular text-[15px] ${color} truncate`}>{value}</div>
    </div>
  );
}
