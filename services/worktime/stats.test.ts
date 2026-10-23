import { describe, expect, test } from "bun:test";
import { compareVersions, firstDay, lastFinishedDay, publish, toCsv, DATASETS } from "./src/stats";

const cell = (month: string, version: string, channel: string, active: number) => ({ month, version, channel, active });

describe("publish", () => {
  test("hides every figure under 10", () => {
    const stats = publish(
      [
        { day: "2026-10-12", pings: 9 },
        { day: "2026-10-13", pings: 10 },
      ],
      [cell("2026-10", "0.2.0", "direct", 9)],
      "2026-10-13",
    );
    expect(stats.months).toEqual([{ month: "2026-10", active: null, averageDaily: null, peakDaily: 10, partial: true }]);
    expect(stats.days).toEqual([
      { day: "2026-10-12", installations: null },
      { day: "2026-10-13", installations: 10 },
    ]);
    expect(stats.details).toEqual([{ month: "2026-10", version: "autres", channel: "tous", active: null }]);
  });

  test("keeps month totals exact and rounds the version and channel cells to 5", () => {
    const stats = publish(
      [],
      [cell("2027-03", "1.2.0", "direct", 162), cell("2027-03", "1.2.0", "zip", 12), cell("2027-03", "1.1.2", "direct", 87), cell("2027-03", "1.0.4", "zip", 4), cell("2027-03", "1.0.3", "zip", 3), cell("2027-03", "1.0.1", "direct", 5)],
      "2027-04-14",
    );
    expect(stats.months[0]?.active).toBe(273);
    expect(stats.details).toEqual([
      { month: "2027-03", version: "1.2.0", channel: "direct", active: 160 },
      { month: "2027-03", version: "1.2.0", channel: "zip", active: 10 },
      { month: "2027-03", version: "1.1.2", channel: "direct", active: 85 },
      { month: "2027-03", version: "autres", channel: "tous", active: 10 },
    ]);
  });

  test("averages a finished month over all its days, and the running one over the days so far", () => {
    const stats = publish(
      [
        { day: "2027-03-02", pings: 310 },
        { day: "2027-04-01", pings: 300 },
      ],
      [cell("2027-03", "1.2.0", "direct", 300), cell("2027-04", "1.2.0", "direct", 300)],
      "2027-04-02",
    );
    expect(stats.months.map((m) => [m.month, m.averageDaily, m.partial])).toEqual([
      ["2027-03", 10, false],
      ["2027-04", 150, true],
    ]);
  });

  test("leads with the last finished month, or the running one before any month ends", () => {
    const cells = [cell("2027-03", "1.2.0", "direct", 40), cell("2027-04", "1.2.0", "direct", 30)];
    expect(publish([], cells, "2027-04-14").reference?.month.month).toBe("2027-03");
    expect(publish([], cells.slice(1), "2027-04-14").reference?.month.month).toBe("2027-04");
    expect(publish([], [], "2027-04-14").reference).toBeNull();
  });

  test("a month that ends on `through` is finished", () => {
    expect(publish([], [cell("2027-03", "1.2.0", "direct", 40)], "2027-03-31").months[0]?.partial).toBe(false);
  });

  test("splits the reference month by channel and version, small keys merged", () => {
    const stats = publish(
      [],
      [cell("2027-03", "1.2.0", "direct", 248), cell("2027-03", "1.1.0", "zip", 41), cell("2027-03", "1.0.0", "zip", 3), cell("2027-03", "0.9.0", "zip", 4)],
      "2027-04-14",
    );
    expect(stats.reference?.byChannel).toEqual({ shares: [{ key: "direct", active: 250 }, { key: "zip", active: 50 }], others: null });
    expect(stats.reference?.byVersion).toEqual({ shares: [{ key: "1.2.0", active: 250 }, { key: "1.1.0", active: 40 }], others: { count: 2, active: null } });
  });
});

test("the figures stop at yesterday and go back to the start of the 12th month", () => {
  expect(lastFinishedDay(new Date("2027-04-15T00:30:00Z"))).toBe("2027-04-14");
  expect(lastFinishedDay(new Date("2027-03-01T10:00:00Z"))).toBe("2027-02-28");
  expect(firstDay("2027-04-14")).toBe("2026-05-01");
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
  const stats = publish([{ day: "2027-04-13", pings: 4 }], [cell("2027-04", "1.2.0", "direct", 40)], "2027-04-14");
  expect(toCsv(DATASETS.quotidien.columns, DATASETS.quotidien.rows(stats))).toBe("jour,installations\n2027-04-13,\n");
  expect(toCsv(DATASETS.mensuel.columns, DATASETS.mensuel.rows(stats))).toBe(
    "mois,actives,moyenne_jour,pic_jour,partiel\n2027-04,40,,,oui\n",
  );
  expect(toCsv(DATASETS.details.columns, [])).toBe("mois,version,mode,actives\n");
});
