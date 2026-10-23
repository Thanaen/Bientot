import type { Client } from "@libsql/client";
import type { Ping } from "./ping";

// One row per day, version and channel: counters only, nothing that identifies a person.
const SCHEMA = `CREATE TABLE IF NOT EXISTS usage_daily (
  day TEXT NOT NULL,
  version TEXT NOT NULL,
  channel TEXT NOT NULL,
  pings INTEGER NOT NULL DEFAULT 0,
  first_of_month INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, version, channel)
)`;

export interface MonthStats {
  month: string;
  activeThisMonth: number;
  averageDaily: number;
  peakDaily: number;
  byChannel: Record<string, number>;
  byVersion: Record<string, number>;
}

export class UsageStore {
  #ready: Promise<unknown> | undefined;

  constructor(private readonly db: Client) {}

  #migrate(): Promise<unknown> {
    this.#ready ??= this.db.execute(SCHEMA).catch((error) => {
      this.#ready = undefined;
      throw error;
    });
    return this.#ready;
  }

  async record(ping: Ping, day: string): Promise<void> {
    await this.#migrate();
    const first = ping.firstThisMonth ? 1 : 0;
    await this.db.execute({
      sql: `INSERT INTO usage_daily (day, version, channel, pings, first_of_month)
            VALUES (?, ?, ?, 1, ?)
            ON CONFLICT (day, version, channel) DO UPDATE SET
              pings = pings + 1,
              first_of_month = first_of_month + excluded.first_of_month`,
      args: [day, ping.version, ping.channel, first],
    });
  }

  // The `count` calendar months up to `now` (UTC). "Active this month" counts first-of-month
  // signals; daily figures count every signal, over every day of the month so far.
  async months(now: Date, count: number): Promise<MonthStats[]> {
    await this.#migrate();
    const cutoff = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - (count - 1), 1));
    const { rows } = await this.db.execute({
      sql: `SELECT day, version, channel, pings, first_of_month FROM usage_daily WHERE day >= ?`,
      args: [cutoff.toISOString().slice(0, 10)],
    });
    const currentMonth = now.toISOString().slice(0, 7);
    const daysCounted = (month: string) => {
      if (month === currentMonth) return now.getUTCDate();
      const [year, monthNumber] = month.split("-").map(Number);
      return new Date(Date.UTC(year!, monthNumber!, 0)).getUTCDate();
    };

    type Month = { first: number; days: Map<string, number>; byChannel: Record<string, number>; byVersion: Record<string, number> };
    const add = (counts: Record<string, number>, key: string, n: number) => {
      counts[key] = (counts[key] ?? 0) + n;
    };
    const months = new Map<string, Month>();
    for (const row of rows) {
      const day = String(row.day);
      const month = day.slice(0, 7);
      let entry = months.get(month);
      if (!entry) months.set(month, (entry = { first: 0, days: new Map(), byChannel: {}, byVersion: {} }));
      const first = Number(row.first_of_month);
      entry.first += first;
      entry.days.set(day, (entry.days.get(day) ?? 0) + Number(row.pings));
      add(entry.byChannel, String(row.channel), first);
      add(entry.byVersion, String(row.version), first);
    }

    return [...months.entries()]
      .sort(([a], [b]) => b.localeCompare(a))
      .map(([month, entry]) => {
        const daily = [...entry.days.values()];
        return {
          month,
          activeThisMonth: entry.first,
          averageDaily: Math.round((10 * daily.reduce((sum, n) => sum + n, 0)) / daysCounted(month)) / 10,
          peakDaily: Math.max(...daily),
          byChannel: entry.byChannel,
          byVersion: entry.byVersion,
        };
      });
  }
}
