import { describe, expect, test } from "bun:test";
import { ENDPOINT, PostHogStore, totalsFromRows } from "./src/store";

type Call = { url: string; init: RequestInit };

function fakePostHog(answer: (call: Call) => Response) {
  const calls: Call[] = [];
  const fetch = (async (url: string, init: RequestInit) => {
    const call = { url: String(url), init };
    calls.push(call);
    return answer(call);
  }) as unknown as typeof globalThis.fetch;
  return { calls, fetch };
}

const store = (fetch: typeof globalThis.fetch, apiKey = "phx_test") =>
  new PostHogStore({ projectToken: "phc_test", projectId: "42", apiKey, fetch });
const keyless = (fetch: typeof globalThis.fetch) => new PostHogStore({ projectToken: "phc_test", projectId: "42", apiKey: undefined, fetch });

describe("PostHogStore.record", () => {
  test("sends the three fields, dated at the day's midnight, with no person, address or reused id", async () => {
    const posthog = fakePostHog(() => new Response("{}"));
    const s = store(posthog.fetch);
    await s.record({ version: "1.0.0", channel: "direct", firstThisMonth: true }, "2026-10-23");
    await s.record({ version: "1.0.0", channel: "direct", firstThisMonth: false }, "2026-10-23");

    expect(posthog.calls.map((c) => c.url)).toEqual(["https://eu.i.posthog.com/i/v0/e/", "https://eu.i.posthog.com/i/v0/e/"]);
    const [first, second] = posthog.calls.map((c) => JSON.parse(String(c.init.body)));
    expect(first).toEqual({
      api_key: "phc_test",
      event: "usage_ping",
      distinct_id: expect.any(String),
      timestamp: "2026-10-23T00:00:00Z",
      properties: {
        version: "1.0.0",
        channel: "direct",
        first_this_month: true,
        $process_person_profile: false,
        $ip: null,
        $geoip_disable: true,
      },
    });
    expect(second.distinct_id).not.toBe(first.distinct_id);
    expect(posthog.calls[0]!.init.headers).toEqual({ "content-type": "application/json" });
  });

  test("fails when PostHog refuses the event, so that the app sends it again later", async () => {
    const posthog = fakePostHog(() => new Response(null, { status: 503 }));
    await expect(store(posthog.fetch).record({ version: "1.0.0", channel: "zip", firstThisMonth: false }, "2026-10-23")).rejects.toThrow();
  });
});

describe("PostHogStore.totals", () => {
  const rows = {
    columns: ["day", "version", "channel", "pings", "first_of_month"],
    results: [
      ["2026-09-30", "0.1.0", "zip", 4, 1],
      ["2026-10-01", "0.2.0", "direct", 12, 12],
      ["2026-10-01", "0.2.0", "zip", 3, 3],
      ["2026-10-02", "0.2.0", "direct", 11, 0],
      ["2026-10-23", "0.2.0", "direct", 9, 0],
    ],
  };

  test("runs the pinned endpoint version with the read key, and sums by day and by month cell", async () => {
    const posthog = fakePostHog(() => Response.json(rows));
    const totals = await store(posthog.fetch).totals("2026-10-01", "2026-10-22");

    expect(posthog.calls).toHaveLength(1);
    expect(posthog.calls[0]!.url).toBe(`https://eu.posthog.com/api/projects/42/endpoints/${ENDPOINT.name}/run?version=${ENDPOINT.version}`);
    expect((posthog.calls[0]!.init.headers as Record<string, string>).authorization).toBe("Bearer phx_test");
    expect(totals).toEqual({
      days: [
        { day: "2026-10-01", pings: 15 },
        { day: "2026-10-02", pings: 11 },
      ],
      cells: [
        { month: "2026-10", version: "0.2.0", channel: "direct", active: 12 },
        { month: "2026-10", version: "0.2.0", channel: "zip", active: 3 },
      ],
    });
  });

  test("reads nothing without a key", async () => {
    const posthog = fakePostHog(() => Response.json(rows));
    await expect(keyless(posthog.fetch).totals("2026-10-01", "2026-10-22")).rejects.toThrow("POSTHOG_PERSONAL_API_KEY");
    expect(posthog.calls).toHaveLength(0);
  });

  test("fails when the endpoint fails, so that the page keeps its last answer", async () => {
    const posthog = fakePostHog(() => new Response(null, { status: 429 }));
    await expect(store(posthog.fetch).totals("2026-10-01", "2026-10-22")).rejects.toThrow("429");
  });
});

test("totalsFromRows keeps only the days asked for", () => {
  const row = (day: string) => ({ day, version: "1.0.0", channel: "store", pings: 1, firstOfMonth: 1 });
  expect(totalsFromRows([row("2026-09-30"), row("2026-10-01"), row("2026-10-02")], "2026-10-01", "2026-10-01").days).toEqual([
    { day: "2026-10-01", pings: 1 },
  ]);
});
