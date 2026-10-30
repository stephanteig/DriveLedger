/** Decimal units, as Finder shows them: `100.18 GB`. */
export function bytes(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  if (unit === 0) return `${n} B`;
  return `${value.toFixed(unit <= 2 ? 1 : 2)} ${units[unit]}`;
}

/** `1156` → `1,156`. */
export function count(n: number): string {
  return n.toLocaleString("en-US");
}

/** RFC 3339 → `YYYY-MM-DD` in local time. */
export function date(rfc3339: string | null | undefined): string {
  if (!rfc3339) return "–";
  const d = new Date(rfc3339);
  if (Number.isNaN(d.getTime())) return rfc3339;
  const pad = (v: number) => String(v).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Nanoseconds since the epoch → `YYYY-MM-DD`. */
export function dateFromNanos(nanos: number | null | undefined): string {
  if (nanos == null) return "–";
  return date(new Date(nanos / 1e6).toISOString());
}

export function duration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return m > 0 ? `${m}m ${String(s % 60).padStart(2, "0")}s` : `${s}s`;
}
