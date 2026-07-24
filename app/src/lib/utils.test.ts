import { describe, it, expect } from "vitest";
import { formatDelta, formatToken } from "./utils";

describe("formatToken", () => {
  it("never shows more than 2 decimals, even for sub-1 amounts", () => {
    expect(formatToken(0.012345, "")).toBe("0.01");
    expect(formatToken(0.0012345, "")).toBe("0.00");
  });

  it("formats amounts at or above 1 with exactly 2 decimals", () => {
    expect(formatToken(1, "")).toBe("1.00");
    expect(formatToken(42.5, "")).toBe("42.50");
  });

  it("compacts thousands/millions with a 2-decimal figure", () => {
    expect(formatToken(1500, "")).toBe("1.50K");
    expect(formatToken(2_500_000, "")).toBe("2.50M");
  });

  it("appends the symbol when one is given", () => {
    expect(formatToken(5, "NEB")).toBe("5.00 NEB");
  });
});

describe("formatDelta", () => {
  it("formats a positive change with a leading plus sign", () => {
    expect(formatDelta(12.4, 7)).toBe("+12%/7d");
  });

  it("formats a negative change with a minus sign, no double sign", () => {
    expect(formatDelta(-8.2, 7)).toBe("-8%/7d");
  });

  it("rounds to the nearest whole percent", () => {
    expect(formatDelta(12.6, 7)).toBe("+13%/7d");
    expect(formatDelta(12.4, 7)).toBe("+12%/7d");
  });

  it("shows a genuine zero change rather than hiding it", () => {
    expect(formatDelta(0, 7)).toBe("+0%/7d");
  });

  it("uses the given interval in the label", () => {
    expect(formatDelta(5, 30)).toBe("+5%/30d");
  });

  it("returns null (no baseline to compare against) instead of a fake 0%", () => {
    expect(formatDelta(null, 7)).toBeNull();
  });
});
