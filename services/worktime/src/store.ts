import type { Ping } from "./ping";
import type { DayTotal, MonthCell } from "./stats";

// Where the signals go and where the public figures come from.
export interface UsageStore {
  record(ping: Ping, day: string): Promise<void>;
  // Raw totals of the days from `from` to `through`: one per day, and one per month, version and channel.
  totals(from: string, through: string): Promise<{ days: DayTotal[]; cells: MonthCell[] }>;
}

// One row per day, version and channel, as the `usage_daily` endpoint answers.
export interface DailyRow {
  day: string;
  version: string;
  channel: string;
  pings: number;
  firstOfMonth: number;
}

export function totalsFromRows(rows: DailyRow[], from: string, through: string): { days: DayTotal[]; cells: MonthCell[] } {
  const days = new Map<string, number>();
  const cells = new Map<string, MonthCell>();
  for (const row of rows) {
    if (row.day < from || row.day > through) continue;
    days.set(row.day, (days.get(row.day) ?? 0) + row.pings);
    const month = row.day.slice(0, 7);
    const key = `${month} ${row.version} ${row.channel}`;
    const cell = cells.get(key) ?? { month, version: row.version, channel: row.channel, active: 0 };
    cell.active += row.firstOfMonth;
    cells.set(key, cell);
  }
  return { days: [...days].map(([day, pings]) => ({ day, pings })), cells: [...cells.values()] };
}

export interface PostHogOptions {
  // The project token, which only lets one send events.
  projectToken: string;
  projectId: string;
  // A project secret API key limited to `endpoint:read`; without it, no figures are read.
  apiKey: string | undefined;
  ingestUrl?: string;
  apiUrl?: string;
  fetch?: typeof fetch;
}

// The saved query behind the figures, in PostHog (README.md, "Statistics"), pinned to one version.
export const ENDPOINT = { name: "usage_daily", version: 1 };

// The signals become `usage_ping` events in PostHog EU. Each event carries the three fields of the
// signal and nothing else: no person profile, no IP address (the request comes from this function,
// and the project discards addresses anyway), a fresh random id, and the day's midnight as its time,
// so that the signals of one day cannot be told apart.
export class PostHogStore implements UsageStore {
  readonly #options: Required<Omit<PostHogOptions, "apiKey">> & { apiKey: string | undefined };

  constructor(options: PostHogOptions) {
    this.#options = {
      ingestUrl: "https://eu.i.posthog.com",
      apiUrl: "https://eu.posthog.com",
      fetch: globalThis.fetch,
      ...options,
    };
  }

  async record(ping: Ping, day: string): Promise<void> {
    const { ingestUrl, projectToken, fetch } = this.#options;
    const response = await fetch(`${ingestUrl}/i/v0/e/`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        api_key: projectToken,
        event: "usage_ping",
        distinct_id: crypto.randomUUID(),
        timestamp: `${day}T00:00:00Z`,
        properties: {
          version: ping.version,
          channel: ping.channel,
          first_this_month: ping.firstThisMonth,
          $process_person_profile: false,
          $ip: null,
          $geoip_disable: true,
        },
      }),
      signal: AbortSignal.timeout(5000),
    });
    if (!response.ok) throw new Error(`PostHog capture answered ${response.status}`);
  }

  async totals(from: string, through: string): Promise<{ days: DayTotal[]; cells: MonthCell[] }> {
    const { apiUrl, projectId, apiKey, fetch } = this.#options;
    if (!apiKey) throw new Error("POSTHOG_PERSONAL_API_KEY is not set");
    const response = await fetch(`${apiUrl}/api/projects/${projectId}/endpoints/${ENDPOINT.name}/run?version=${ENDPOINT.version}`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${apiKey}` },
      body: JSON.stringify({ refresh: "cache", limit: 10000 }),
      signal: AbortSignal.timeout(20000),
    });
    if (!response.ok) throw new Error(`PostHog endpoint answered ${response.status}`);
    const { columns, results } = (await response.json()) as { columns: string[]; results: unknown[][] };
    const at = (name: string) => columns.indexOf(name);
    const [day, version, channel, pings, first] = ["day", "version", "channel", "pings", "first_of_month"].map(at);
    const rows = results.map((row) => ({
      day: String(row[day!]),
      version: String(row[version!]),
      channel: String(row[channel!]),
      pings: Number(row[pings!]),
      firstOfMonth: Number(row[first!]),
    }));
    return totalsFromRows(rows, from, through);
  }
}

// The signals kept in memory, for tests and `bun run dev`.
export class MemoryStore implements UsageStore {
  readonly rows: DailyRow[] = [];

  async record(ping: Ping, day: string): Promise<void> {
    let row = this.rows.find((r) => r.day === day && r.version === ping.version && r.channel === ping.channel);
    if (!row) this.rows.push((row = { day, version: ping.version, channel: ping.channel, pings: 0, firstOfMonth: 0 }));
    row.pings += 1;
    if (ping.firstThisMonth) row.firstOfMonth += 1;
  }

  async totals(from: string, through: string): Promise<{ days: DayTotal[]; cells: MonthCell[] }> {
    return totalsFromRows(this.rows, from, through);
  }
}
