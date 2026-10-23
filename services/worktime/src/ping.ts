// The daily signal WorkTime sends, and the only shape the service accepts.
// README.md says what is sent and what is kept.

export const CHANNELS = ["store", "direct", "zip"] as const;
export type Channel = (typeof CHANNELS)[number];

export interface Ping {
  version: string;
  channel: Channel;
  firstThisMonth: boolean;
}

// 1.0.0 or 1.0.0-alpha.3; bounded so nobody can store free text through it.
const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,3}(-[0-9A-Za-z.]{1,20})?$/;
const KEYS = ["channel", "first_this_month", "version"];

export const MAX_BODY_BYTES = 256;

export function parsePing(body: unknown): Ping | null {
  if (typeof body !== "object" || body === null || Array.isArray(body)) return null;
  const record = body as Record<string, unknown>;
  if (Object.keys(record).sort().join() !== KEYS.join()) return null;

  const { version, channel, first_this_month } = record;
  if (typeof version !== "string" || !VERSION.test(version)) return null;
  if (!CHANNELS.includes(channel as Channel)) return null;
  if (typeof first_this_month !== "boolean") return null;

  return { version, channel: channel as Channel, firstThisMonth: first_this_month };
}

export function utcDay(now: Date): string {
  return now.toISOString().slice(0, 10);
}
