// The public statistics: what the counters in usage_daily may show to anyone, and in which shape.
// Publication rules (README.md, "Statistics"): only finished UTC days, no figure under 10, and the
// version and channel figures rounded to the nearest 5 so that a hidden one cannot be found back by
// subtracting the others from a total.

export const THRESHOLD = 10;
const STEP = 5;
export const MONTHS = 12;

// What the store hands over: one row per finished day, and one per month, version and channel.
export interface DayTotal {
  day: string;
  pings: number;
}
export interface MonthCell {
  month: string;
  version: string;
  channel: string;
  active: number;
}

export interface PublicMonth {
  month: string;
  active: number | null;
  averageDaily: number | null;
  peakDaily: number | null;
  // The month is still running: its figures cover the days up to `through` only.
  partial: boolean;
}
export interface PublicDay {
  day: string;
  installations: number | null;
}
export interface PublicDetail {
  month: string;
  // "autres" gathers the versions under the threshold, with channel "tous".
  version: string;
  channel: string;
  active: number | null;
}
export interface Share {
  key: string;
  active: number;
}
export interface Breakdown {
  shares: Share[];
  // Keys under the threshold, merged; null when there are none.
  others: { count: number; active: number | null } | null;
}
export interface PublicStats {
  // The last finished UTC day the figures cover.
  through: string;
  months: PublicMonth[];
  days: PublicDay[];
  details: PublicDetail[];
  // The month the page leads with: the last finished one, or the running one before any month ends.
  reference: { month: PublicMonth; byVersion: Breakdown; byChannel: Breakdown } | null;
}

const shown = (n: number) => (n < THRESHOLD ? null : n);
const rounded = (n: number) => (n < THRESHOLD ? null : Math.max(THRESHOLD, Math.round(n / STEP) * STEP));

export function lastFinishedDay(now: Date): string {
  return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - 1)).toISOString().slice(0, 10);
}

// The first day of the oldest month shown, MONTHS months up to the one holding `through`.
export function firstDay(through: string): string {
  const [year, month] = through.split("-").map(Number);
  return new Date(Date.UTC(year!, month! - MONTHS, 1)).toISOString().slice(0, 10);
}

const daysInMonth = (month: string) => {
  const [year, number] = month.split("-").map(Number);
  return new Date(Date.UTC(year!, number!, 0)).getUTCDate();
};

export function publish(days: DayTotal[], cells: MonthCell[], through: string): PublicStats {
  const throughMonth = through.slice(0, 7);
  const months = new Map<string, { active: number; pings: number; peak: number }>();
  for (const cell of cells) {
    const entry = months.get(cell.month) ?? { active: 0, pings: 0, peak: 0 };
    entry.active += cell.active;
    months.set(cell.month, entry);
  }
  for (const { day, pings } of days) {
    const entry = months.get(day.slice(0, 7)) ?? { active: 0, pings: 0, peak: 0 };
    entry.pings += pings;
    entry.peak = Math.max(entry.peak, pings);
    months.set(day.slice(0, 7), entry);
  }

  const publicMonths = [...months.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([month, { active, pings, peak }]) => {
      const partial = month === throughMonth && Number(through.slice(8)) < daysInMonth(month);
      const daysCounted = partial ? Number(through.slice(8)) : daysInMonth(month);
      return {
        month,
        active: shown(active),
        averageDaily: shown(Math.round(pings / daysCounted)),
        peakDaily: shown(peak),
        partial,
      };
    });

  const details: PublicDetail[] = [];
  for (const month of [...new Set(cells.map((cell) => cell.month))].sort()) {
    let others = 0;
    const kept: PublicDetail[] = [];
    for (const cell of cells.filter((c) => c.month === month)) {
      if (cell.active < THRESHOLD) others += cell.active;
      else kept.push({ month, version: cell.version, channel: cell.channel, active: rounded(cell.active) });
    }
    kept.sort((a, b) => compareVersions(b.version, a.version) || a.channel.localeCompare(b.channel));
    details.push(...kept);
    if (others > 0) details.push({ month, version: "autres", channel: "tous", active: rounded(others) });
  }

  const month = publicMonths.findLast((m) => !m.partial) ?? publicMonths.at(-1);
  return {
    through,
    months: publicMonths,
    days: [...days].sort((a, b) => a.day.localeCompare(b.day)).map(({ day, pings }) => ({ day, installations: shown(pings) })),
    details,
    reference: month
      ? { month, byVersion: breakdown(cells, month.month, "version"), byChannel: breakdown(cells, month.month, "channel") }
      : null,
  };
}

// The actives of one month split by version or by channel, small keys merged, the rest rounded.
function breakdown(cells: MonthCell[], month: string, by: "version" | "channel"): Breakdown {
  const totals = new Map<string, number>();
  for (const cell of cells) if (cell.month === month) totals.set(cell[by], (totals.get(cell[by]) ?? 0) + cell.active);
  const shares: Share[] = [];
  let count = 0;
  let small = 0;
  for (const [key, active] of totals) {
    if (active < THRESHOLD) {
      count += 1;
      small += active;
    } else shares.push({ key, active: rounded(active)! });
  }
  shares.sort((a, b) => b.active - a.active || (by === "version" ? compareVersions(b.key, a.key) : a.key.localeCompare(b.key)));
  return { shares, others: count > 0 ? { count, active: rounded(small) } : null };
}

// 1.10.0 after 1.9.2, and 1.0.0 after 1.0.0-alpha.3.
export function compareVersions(a: string, b: string): number {
  const parse = (v: string) => {
    const [core = "", pre] = v.split("-", 2);
    return { parts: core.split(".").map(Number), pre };
  };
  const x = parse(a);
  const y = parse(b);
  for (let i = 0; i < 3; i++) {
    const d = (x.parts[i] ?? 0) - (y.parts[i] ?? 0);
    if (d) return d;
  }
  if (x.pre === y.pre) return 0;
  if (x.pre === undefined) return 1;
  if (y.pre === undefined) return -1;
  return x.pre.localeCompare(y.pre, "en", { numeric: true });
}

// ---- Open data files ----

type Value = string | number | boolean | null;
type Row = Record<string, Value>;

// Each file: its columns, in order, and its rows. Values under the threshold are null (empty in CSV).
export const DATASETS = {
  mensuel: {
    columns: ["mois", "actives", "moyenne_jour", "pic_jour", "partiel"],
    rows: (s: PublicStats): Row[] =>
      s.months.map((m) => ({ mois: m.month, actives: m.active, moyenne_jour: m.averageDaily, pic_jour: m.peakDaily, partiel: m.partial })),
  },
  quotidien: {
    columns: ["jour", "installations"],
    rows: (s: PublicStats): Row[] => s.days.map((d) => ({ jour: d.day, installations: d.installations })),
  },
  details: {
    columns: ["mois", "version", "mode", "actives"],
    rows: (s: PublicStats): Row[] => s.details.map((d) => ({ mois: d.month, version: d.version, mode: d.channel, actives: d.active })),
  },
} as const;
export type Dataset = keyof typeof DATASETS;

export function toCsv(columns: readonly string[], rows: Row[]): string {
  const cell = (value: Value) => (value === null ? "" : typeof value === "boolean" ? (value ? "oui" : "non") : String(value));
  return [columns.join(","), ...rows.map((row) => columns.map((c) => cell(row[c] ?? null)).join(","))].join("\n") + "\n";
}
