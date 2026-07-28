import { describe, it, expect } from "vitest";
import { findNextSchema, findConfirmSchema } from "./validation";

const validAnswer = {
  facet: { kind: "tag", value: "lending" },
  value: "yes",
};

describe("findNextSchema", () => {
  it("accepts an empty body — the funnel's first turn has no answers yet", () => {
    expect(findNextSchema.parse({})).toEqual({ answers: [], forceResults: false });
  });

  it("round-trips a well-formed answer unchanged", () => {
    const input = { answers: [{ facet: { kind: "tag", value: "lending" }, value: "skip" }] };
    expect(findNextSchema.parse(input)).toEqual({
      answers: [{ facet: { kind: "tag", value: "lending" }, value: "skip" }],
      forceResults: false,
    });
  });

  it("rejects a facet kind outside category/chain/tag", () => {
    const result = findNextSchema.safeParse({
      answers: [{ facet: { kind: "protocol", value: "x" }, value: "yes" }],
    });
    expect(result.success).toBe(false);
  });

  it("rejects a hedged answer — 'probably' is deliberately outside the v1 vocabulary (A15: no citable likelihood exists for it)", () => {
    const result = findNextSchema.safeParse({
      answers: [{ facet: { kind: "tag", value: "x" }, value: "probably" }],
    });
    expect(result.success).toBe(false);
  });

  it("rejects more than 16 answers", () => {
    const result = findNextSchema.safeParse({ answers: new Array(17).fill(validAnswer) });
    expect(result.success).toBe(false);
  });
});

describe("findConfirmSchema", () => {
  it("rejects an outcome outside confirmed/rejected/clicked", () => {
    const result = findConfirmSchema.safeParse({ answers: [], appId: "a", outcome: "maybe" });
    expect(result.success).toBe(false);
  });

  it("defaults answers to [] when the body omits them", () => {
    expect(findConfirmSchema.parse({ appId: "a", outcome: "confirmed" })).toEqual({
      answers: [],
      appId: "a",
      outcome: "confirmed",
    });
  });

  it("strips a client-supplied visitorId rather than honouring it", () => {
    const result = findConfirmSchema.safeParse({
      answers: [],
      appId: "a",
      outcome: "confirmed",
      visitorId: "spoofed",
    });
    expect(result.success).toBe(true);
    expect(result.success && "visitorId" in result.data).toBe(false);
  });
});
