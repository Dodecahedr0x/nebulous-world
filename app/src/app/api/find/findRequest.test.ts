import { describe, expect, it } from "vitest";
import type { FindAnswer } from "@/lib/types";
import type { FindConfirmRequest } from "@/lib/validation";
import {
  MAX_FORWARDED_ANSWERS,
  dedupeAnswers,
  mayRecordOutcome,
  toFindConfirmInput,
  toFindNextInput,
} from "./findRequest";

function answer(
  kind: FindAnswer["facet"]["kind"],
  value: string,
  answerValue: FindAnswer["value"],
): FindAnswer {
  return { facet: { kind, value }, value: answerValue };
}

describe("dedupeAnswers", () => {
  it("returns an empty list unchanged", () => {
    expect(dedupeAnswers([])).toEqual([]);
  });

  it("keeps one entry per facet, carrying the last value", () => {
    const result = dedupeAnswers([answer("tag", "lending", "yes"), answer("tag", "lending", "no")]);
    expect(result).toHaveLength(1);
    expect(result[0].value).toBe("no");
  });

  it("does not merge facets that differ only by kind", () => {
    const result = dedupeAnswers([answer("category", "defi", "yes"), answer("tag", "defi", "no")]);
    expect(result).toEqual([answer("category", "defi", "yes"), answer("tag", "defi", "no")]);
  });

  it("orders by first appearance, not by last write", () => {
    const result = dedupeAnswers([
      answer("category", "defi", "yes"),
      answer("chain", "solana", "yes"),
      answer("category", "defi", "no"),
    ]);
    expect(result.map((a) => a.facet.value)).toEqual(["defi", "solana"]);
    expect(result[0].value).toBe("no");
  });
});

describe("toFindNextInput", () => {
  it("dedupes the answer history and preserves forceResults", () => {
    const input = toFindNextInput({
      answers: [
        answer("tag", "lending", "yes"),
        answer("tag", "lending", "skip"),
        answer("chain", "solana", "yes"),
      ],
      forceResults: false,
    });
    expect(input.answers).toEqual([answer("tag", "lending", "skip"), answer("chain", "solana", "yes")]);
    expect(input.forceResults).toBe(false);
  });

  it("never forwards more than MAX_FORWARDED_ANSWERS answers", () => {
    const answers = Array.from({ length: MAX_FORWARDED_ANSWERS + 9 }, (_, i) =>
      answer("tag", `tag-${i}`, "yes"),
    );
    expect(toFindNextInput({ answers, forceResults: true }).answers).toHaveLength(MAX_FORWARDED_ANSWERS);
  });
});

describe("toFindConfirmInput", () => {
  const body: FindConfirmRequest = {
    answers: [answer("tag", "lending", "yes")],
    appId: "app-1",
    outcome: "confirmed",
    turnstileToken: "tok",
  };

  // A28: visitor identity is server-derived, never taken from the body — a
  // client-chosen sessionId would make the confirm endpoint's idempotency key
  // attacker-chosen, i.e. free confirmation farming.
  it("A28: uses the server-derived visitor and ignores a spoofed body identity", () => {
    const spoofed = { ...body, visitorId: "spoofed-v", sessionId: "spoofed-s" } as FindConfirmRequest;
    const input = toFindConfirmInput(spoofed, { visitorId: "server-v", sessionId: "server-s" });
    expect(input.visitorId).toBe("server-v");
    expect(input.sessionId).toBe("server-s");
  });

  it("does not carry turnstileToken through to the indexer", () => {
    const input = toFindConfirmInput(body, { visitorId: "server-v", sessionId: "server-s" });
    expect(Object.keys(input)).not.toContain("turnstileToken");
  });

  it("forwards the deduped answers, appId and outcome", () => {
    const input = toFindConfirmInput(
      { ...body, answers: [answer("tag", "lending", "yes"), answer("tag", "lending", "no")] },
      { visitorId: "server-v", sessionId: "server-s" },
    );
    expect(input.answers).toEqual([answer("tag", "lending", "no")]);
    expect(input.appId).toBe("app-1");
    expect(input.outcome).toBe("confirmed");
  });
});

describe("mayRecordOutcome", () => {
  // A4/A29: with no TURNSTILE_SECRET_KEY the verifier always returns false, so
  // gating on it alone would break /find in local and simulation mode.
  it("records the outcome when Turnstile is unconfigured", () => {
    expect(mayRecordOutcome(false, false)).toBe(true);
  });

  it("rejects a failed token only when Turnstile is configured", () => {
    expect(mayRecordOutcome(true, false)).toBe(false);
    expect(mayRecordOutcome(true, true)).toBe(true);
  });
});
