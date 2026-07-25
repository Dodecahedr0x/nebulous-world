/** True for HTTP 429 (rate limited) — the only case worth retrying here;
 * other RPC failures (bad instruction, insufficient funds, ...) should
 * fail fast instead of being retried. web3.js/fetch don't expose a typed
 * status code for this failure path, so match on the message text. */
function isRateLimited(err: unknown): boolean {
  const message = err instanceof Error ? err.message : String(err);
  return /429|too many requests/i.test(message);
}

/**
 * Runs `fn`, retrying with exponential backoff (+ jitter) only on HTTP 429s
 * — for RPC calls (`getAccountInfo`, `sendAndConfirm`, ...) against
 * rate-limited endpoints like the public devnet RPC, which
 * `createAppsOnchain.ts` can burst against when creating many apps at once.
 */
export async function withRateLimitRetry<T>(
  fn: () => Promise<T>,
  { maxAttempts = 6, baseDelayMs = 500, maxDelayMs = 15_000 } = {},
): Promise<T> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await fn();
    } catch (err) {
      if (!isRateLimited(err) || attempt >= maxAttempts) throw err;
      const delay = Math.min(maxDelayMs, baseDelayMs * 2 ** (attempt - 1)) * (0.5 + Math.random() * 0.5);
      console.warn(`  … rate limited (429), retrying in ${Math.round(delay)}ms (attempt ${attempt}/${maxAttempts})`);
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  }
}
