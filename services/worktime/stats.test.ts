import { describe, expect, test } from "bun:test";
import { compareVersions, firstDay, lastFinishedDay, publish, toCsv, DATASETS } from "./src/stats";

const cell = (month: string, version: string, channel: string, active: number) => ({ month, version, channel, active });
// A busy day, so that the month's actives stay under the published days' total.
const busy = (month: string) => [{ day: `${month}-15`, pings: 1000 }];
const detailsCsv = (stats: ReturnType<typeof publish>) => toCsv(DATASETS.details.columns, DATASETS.details.rows(stats));

describe("publish", () => {
  test("hides every figure under 10", () => {
    const stats = publish(
      [
        { day: "2026-10-12", pings: 9 },
        { day: "2026-10-13", pings: 10 },
      ],
      [cell("2026-10", "0.2.0", "direct", 9)],
      "2026-10-31",
    );
    expect(stats.months).toEqual([
      {
        month: "2026-10",
        active: null,
        averageDaily: null,
        peakDaily: 10,
        byVersion: { shares: [], others: { count: 1, active: null } },
        byChannel: { shares: [], others: { count: 1, active: null } },
      },
    ]);
    expect(stats.days).toEqual([
      { day: "2026-10-12", installations: null },
      { day: "2026-10-13", installations: 10 },
    ]);
  });

  test("keeps month totals exact and rounds the version and channel figures to 5", () => {
    const stats = publish(
      busy("2027-03"),
      [cell("2027-03", "1.2.0", "direct", 162), cell("2027-03", "1.2.0", "zip", 12), cell("2027-03", "1.1.2", "direct", 87), cell("2027-03", "1.0.4", "zip", 4), cell("2027-03", "1.0.3", "zip", 3), cell("2027-03", "1.0.1", "direct", 5)],
      "2027-04-14",
    );
    expect(stats.months[0]?.active).toBe(273);
    expect(stats.months[0]?.byVersion).toEqual({ shares: [{ key: "1.2.0", active: 175 }, { key: "1.1.2", active: 85 }], others: { count: 3, active: 10 } });
    expect(stats.months[0]?.byChannel).toEqual({ shares: [{ key: "direct", active: 255 }, { key: "zip", active: 20 }], others: null });
  });

  // Pullfrog's counterexamples on PR #16: each one let a figure under 10 be worked out.
  test("merges a small key with the smallest shown ones until the merge reaches 10", () => {
    // Exact total 11 with a single shown 10 would give the hidden 1 away.
    const stats = publish(busy("2027-03"), [cell("2027-03", "1.2.0", "direct", 10), cell("2027-03", "1.1.0", "direct", 1)], "2027-03-31");
    expect(stats.months[0]?.active).toBe(11);
    expect(stats.months[0]?.byVersion).toEqual({ shares: [], others: { count: 2, active: 10 } });
    // Small keys already at 10 together stay apart from the shown ones.
    expect(
      publish(busy("2027-03"), [cell("2027-03", "1.2.0", "direct", 40), cell("2027-03", "1.1.0", "direct", 6), cell("2027-03", "1.0.0", "direct", 4)], "2027-03-31").months[0]
        ?.byVersion,
    ).toEqual({ shares: [{ key: "1.2.0", active: 40 }], others: { count: 2, active: 10 } });
    // And the merge reaches 10 with the next smallest, not the biggest.
    expect(
      publish(busy("2027-03"), [cell("2027-03", "1.2.0", "direct", 300), cell("2027-03", "1.1.0", "direct", 20), cell("2027-03", "1.0.0", "direct", 3)], "2027-03-31").months[0]
        ?.byVersion,
    ).toEqual({ shares: [{ key: "1.2.0", active: 300 }], others: { count: 2, active: 25 } });
  });

  test("derives the averages and peaks from the published days only", () => {
    // A true average of (20 + 9) over the days would give the hidden 9 away.
    const stats = publish(
      [
        { day: "2027-03-01", pings: 20 },
        { day: "2027-03-02", pings: 9 },
        ...Array.from({ length: 29 }, (_, i) => ({ day: `2027-03-${String(i + 3).padStart(2, "0")}`, pings: 10 })),
      ],
      [cell("2027-03", "1.2.0", "direct", 30)],
      "2027-03-31",
    );
    // (20 + 0 + 29 × 10) / 31 = 10, whatever the hidden day was.
    expect(stats.months[0]?.averageDaily).toBe(10);
    expect(stats.months[0]?.peakDaily).toBe(20);
  });

  test("publishes no monthly figure while the month is running", () => {
    // Two snapshots of a growing total (100, then 109) would give the 9 of the day between away.
    const cells = [cell("2027-03", "1.2.0", "direct", 40), cell("2027-04", "1.2.0", "direct", 100)];
    const stats = publish([{ day: "2027-04-10", pings: 120 }], cells, "2027-04-10");
    expect(stats.months.map((m) => m.month)).toEqual(["2027-03"]);
    expect(stats.days).toEqual([{ day: "2027-04-10", installations: 120 }]);
    expect(detailsCsv(stats)).not.toContain("2027-04");
    expect(publish([], cells, "2027-04-30").months.map((m) => m.month)).toEqual(["2027-03", "2027-04"]);
  });

  test("hides a month whose actives exceed its published days", () => {
    // A shown 10, a hidden day and an exact 19 actives would make the hidden day 9: each first-of-month
    // signal is also one of the day's signals.
    const days = [
      { day: "2027-03-01", pings: 10 },
      { day: "2027-03-02", pings: 9 },
    ];
    const stats = publish(days, [cell("2027-03", "1.2.0", "direct", 19)], "2027-03-31");
    expect(stats.months[0]).toMatchObject({ active: null, byVersion: { shares: [], others: { count: 1, active: null } } });
    expect(stats.days.map((d) => d.installations)).toEqual([10, null]);
    // Once the published days alone cover the actives, the hidden days bound nothing.
    expect(publish([...days, { day: "2027-03-03", pings: 10 }], [cell("2027-03", "1.2.0", "direct", 19)], "2027-03-31").months[0]?.active).toBe(19);
  });

  test("the details file lists each month by version, then by channel", () => {
    const stats = publish(busy("2027-03"), [cell("2027-03", "1.2.0", "direct", 248), cell("2027-03", "1.1.0", "zip", 41), cell("2027-03", "1.0.0", "zip", 6), cell("2027-03", "0.9.0", "zip", 4)], "2027-03-31");
    expect(detailsCsv(stats)).toBe(
      "mois,par,valeur,actives\n2027-03,version,1.2.0,250\n2027-03,version,1.1.0,40\n2027-03,version,autres,10\n2027-03,mode,direct,250\n2027-03,mode,zip,50\n",
    );
  });
});

test("the figures stop at yesterday and go back 12 finished months", () => {
  expect(lastFinishedDay(new Date("2027-04-15T00:30:00Z"))).toBe("2027-04-14");
  expect(lastFinishedDay(new Date("2027-03-01T10:00:00Z"))).toBe("2027-02-28");
  expect(firstDay("2027-04-14")).toBe("2026-04-01");
  expect(firstDay("2027-03-31")).toBe("2026-04-01");
  expect(firstDay("2027-01-05")).toBe("2026-01-01");
});

test("versions sort by number, releases after their pre-releases", () => {
  expect(["1.10.0", "1.9.2", "1.0.0", "1.0.0-alpha.10", "1.0.0-alpha.9"].sort(compareVersions)).toEqual([
    "1.0.0-alpha.9",
    "1.0.0-alpha.10",
    "1.0.0",
    "1.9.2",
    "1.10.0",
  ]);
});

test("CSV leaves hidden figures empty and keeps its header without rows", () => {
  const stats = publish([{ day: "2027-03-13", pings: 4 }], [cell("2027-03", "1.2.0", "direct", 40)], "2027-04-14");
  expect(toCsv(DATASETS.quotidien.columns, DATASETS.quotidien.rows(stats))).toBe("jour,installations\n2027-03-13,\n");
  expect(toCsv(DATASETS.mensuel.columns, DATASETS.mensuel.rows(stats))).toBe("mois,actives,moyenne_jour,pic_jour\n2027-03,,,\n");
  expect(toCsv(DATASETS.details.columns, [])).toBe("mois,par,valeur,actives\n");
});
