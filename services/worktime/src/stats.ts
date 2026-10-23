// The public statistics: what the counters in usage_daily may show to anyone, and in which shape.
// Publication rules (README.md, "Statistics"): only finished UTC days, and monthly figures only for
// finished months, so that no total is published twice while it grows. No figure under 10, and a
// small group is merged with others until the merge reaches 10 too, so that it cannot be found back
// by subtracting the shown figures from a total. Version and channel figures are rounded to 5.

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

export interface Share {
  key: string;
  active: number;
}
export interface Breakdown {
  shares: Share[];
  // The keys under the threshold, merged with the smallest others until the merge reaches it; null
  // when no key is merged.
  others: { count: number; active: number | null } | null;
}
export interface PublicMonth {
  month: string;
  active: number | null;
  // Both from the published days, a hidden day counting as 0, so that they reveal nothing more.
  averageDaily: number | null;
  peakDaily: number | null;
  byVersion: Breakdown;
  byChannel: Breakdown;
}
export interface PublicDay {
  day: string;
  installations: number | null;
}
export interface PublicStats {
  // The last finished UTC day the figures cover.
  through: string;
  // The finished months, oldest first; the page leads with the last one.
  months: PublicMonth[];
  days: PublicDay[];
}

const shown = (n: number) => (n < THRESHOLD ? null : n);
const rounded = (n: number) => (n < THRESHOLD ? null : Math.max(THRESHOLD, Math.round(n / STEP) * STEP));

export function lastFinishedDay(now: Date): string {
  return new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - 1)).toISOString().slice(0, 10);
}

const daysInMonth = (month: string) => {
  const [year, number] = month.split("-").map(Number);
  return new Date(Date.UTC(year!, number!, 0)).getUTCDate();
};

// The last month that ends on or before `through`.
function lastFinishedMonth(through: string): string {
  const [year, month, day] = through.split("-").map(Number);
  const finished = day === daysInMonth(through.slice(0, 7)) ? month! : month! - 1;
  return new Date(Date.UTC(year!, finished - 1, 1)).toISOString().slice(0, 7);
}

// The first day of the oldest month shown: MONTHS finished months, up to the one before `through`.
export function firstDay(through: string): string {
  const [year, month] = lastFinishedMonth(through).split("-").map(Number);
  return new Date(Date.UTC(year!, month! - MONTHS, 1)).toISOString().slice(0, 10);
}

export function publish(days: DayTotal[], cells: MonthCell[], through: string): PublicStats {
  const last = lastFinishedMonth(through);
  const publicDays = [...days].sort((a, b) => a.day.localeCompare(b.day)).map(({ day, pings }) => ({ day, installations: shown(pings) }));
  const months = [...new Set(cells.map((cell) => cell.month))].filter((month) => month <= last).sort();
  return {
    through,
    months: months.map((month) => {
      const own = cells.filter((cell) => cell.month === month);
      const daily = publicDays.filter((d) => d.day.startsWith(month)).map((d) => d.installations ?? 0);
      const published = daily.reduce((sum, n) => sum + n, 0);
      const active = own.reduce((sum, cell) => sum + cell.active, 0);
      // Each first-of-month signal is also one of the day's signals, so the actives cannot exceed the
      // days' total. When they exceed the published days, they tell how much the hidden days hold at
      // least (10 and a hidden day under an exact 19 make that day 9): the month stays hidden.
      const hidden = active > published;
      return {
        month,
        active: hidden ? null : shown(active),
        averageDaily: shown(Math.round(published / daysInMonth(month))),
        peakDaily: shown(Math.max(0, ...daily)),
        byVersion: hidden ? suppressed(own, "version") : breakdown(own, "version"),
        byChannel: hidden ? suppressed(own, "channel") : breakdown(own, "channel"),
      };
    }),
    days: publicDays,
  };
}

// Every key merged into a hidden "autres".
function suppressed(cells: MonthCell[], by: "version" | "channel"): Breakdown {
  return { shares: [], others: { count: new Set(cells.map((cell) => cell[by])).size, active: null } };
}

// The actives of one month split by version or by channel. The keys under the threshold are merged;
// while that merge is under the threshold too, the smallest shown key joins it.
function breakdown(cells: MonthCell[], by: "version" | "channel"): Breakdown {
  const totals = new Map<string, number>();
  for (const cell of cells) totals.set(cell[by], (totals.get(cell[by]) ?? 0) + cell.active);
  const kept = [...totals].sort(([, a], [, b]) => a - b);
  let count = 0;
  let small = 0;
  while (kept.length && (kept[0]![1] < THRESHOLD || (count > 0 && small < THRESHOLD))) {
    small += kept.shift()![1];
    count += 1;
  }
  const shares = kept.map(([key, active]) => ({ key, active: rounded(active)! }));
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

type Value = string | number | null;
type Row = Record<string, Value>;

// Each file: its columns, in order, and its rows. Values under the threshold are null (empty in CSV).
export const DATASETS = {
  mensuel: {
    columns: ["mois", "actives", "moyenne_jour", "pic_jour"],
    rows: (s: PublicStats): Row[] =>
      s.months.map((m) => ({ mois: m.month, actives: m.active, moyenne_jour: m.averageDaily, pic_jour: m.peakDaily })),
  },
  quotidien: {
    columns: ["jour", "installations"],
    rows: (s: PublicStats): Row[] => s.days.map((d) => ({ jour: d.day, installations: d.installations })),
  },
  // One block per month and dimension; "autres" holds the merged keys.
  details: {
    columns: ["mois", "par", "valeur", "actives"],
    rows: (s: PublicStats): Row[] =>
      s.months.flatMap((m) =>
        (
          [
            ["version", m.byVersion],
            ["mode", m.byChannel],
          ] as const
        ).flatMap(([par, { shares, others }]) => [
          ...shares.map((share) => ({ mois: m.month, par, valeur: share.key, actives: share.active })),
          ...(others ? [{ mois: m.month, par, valeur: "autres", actives: others.active }] : []),
        ]),
      ),
  },
} as const;
export type Dataset = keyof typeof DATASETS;

export function toCsv(columns: readonly string[], rows: Row[]): string {
  const cell = (value: Value) => (value === null ? "" : String(value));
  return [columns.join(","), ...rows.map((row) => columns.map((c) => cell(row[c] ?? null)).join(","))].join("\n") + "\n";
}
