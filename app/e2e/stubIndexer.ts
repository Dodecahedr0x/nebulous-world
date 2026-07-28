/**
 * A stand-in for the indexer's HTTP API, for the /find end-to-end tier.
 *
 * The real binary cannot boot here — indexer/src/main.rs requires a live Solana
 * RPC with an initialised Config account — and the gap this tier exists to
 * close (A83) is app-side WIRING, not the engine's answers. So the three
 * endpoints /find needs are stubbed with deterministic, obviously-fake data,
 * and the interesting part is the recorder: every request is kept and served
 * back at GET /__requests, which is what makes "did the server fetch question
 * 1 or not?" an assertion rather than an inference.
 *
 * Run standalone: `npx tsx e2e/stubIndexer.ts` (port from STUB_PORT).
 */
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

export interface RecordedRequest {
  method: string;
  path: string;
  body: unknown;
}

/**
 * One facet per turn, so a spec can tell question 1 from question 2 by reading
 * the screen. Indexed by how many answers the client sent — the funnel is
 * stateless, so `answers.length` is the whole of the server's memory.
 */
export const STUB_FACETS = [
  { kind: "category", value: "defi", prompt: "Are you looking for a DeFi app?" },
  { kind: "tag", value: "lending", prompt: "Do you want to lend or borrow?" },
  { kind: "chain", value: "solana", prompt: "Does it have to be on Solana?" },
  { kind: "tag", value: "yield", prompt: "Are you chasing yield?" },
] as const;

export const STUB_APP = {
  id: "stub-app-1",
  slug: "stub-app",
  name: "Stub App",
  tagline: "A fixture, not a real app.",
  description: "Returned by the /find e2e stub indexer.",
  url: "https://example.invalid/stub",
  iconUrl: null,
  category: "defi",
  chain: "solana",
  status: "approved",
  createdAt: "2026-01-01T00:00:00.000Z",
  submittedBy: null,
  voteCount: 0,
  voteWeight: 0,
  stakeTotal: 0,
  viewCount: 0,
  rankScore: 0,
  tags: [],
};

function findNextResponse(body: unknown) {
  const parsed = (body ?? {}) as { answers?: unknown[]; forceResults?: boolean };
  const asked = Array.isArray(parsed.answers) ? parsed.answers.length : 0;

  if (parsed.forceResults) {
    return {
      question: null,
      shortlist: [{ app: STUB_APP, confidence: 0.9 }],
      candidateCount: 1,
      questionsAsked: asked,
      done: true,
    };
  }

  const facet = STUB_FACETS[asked % STUB_FACETS.length];
  return {
    question: { facet: { kind: facet.kind, value: facet.value }, prompt: facet.prompt },
    shortlist: [],
    candidateCount: 42 - asked,
    questionsAsked: asked,
    done: false,
  };
}

function readBody(req: IncomingMessage): Promise<unknown> {
  return new Promise((resolve) => {
    const chunks: Buffer[] = [];
    req.on("data", (c: Buffer) => chunks.push(c));
    req.on("end", () => {
      const raw = Buffer.concat(chunks).toString("utf8");
      if (!raw) return resolve(null);
      try {
        resolve(JSON.parse(raw));
      } catch {
        resolve(raw);
      }
    });
  });
}

export function createStubIndexer() {
  const recorded: RecordedRequest[] = [];

  const server = createServer((req: IncomingMessage, res: ServerResponse) => {
    void (async () => {
      const path = (req.url ?? "/").split("?")[0];
      const method = req.method ?? "GET";
      const body = method === "GET" ? null : await readBody(req);

      const send = (status: number, payload: unknown) => {
        const json = JSON.stringify(payload);
        res.writeHead(status, {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(json),
        });
        res.end(json);
      };

      // Test-only control surface, deliberately not recorded.
      if (path === "/__requests") return send(200, recorded);
      if (path === "/__reset") {
        recorded.length = 0;
        return send(200, { ok: true });
      }

      recorded.push({ method, path, body });

      if (method === "POST" && path === "/find/next") return send(200, findNextResponse(body));
      if (method === "POST" && path === "/find/confirm") return send(200, { ok: true });
      if (method === "GET" && path === "/find/stats") {
        return send(200, { avgQuestionsToConfirm: null });
      }
      return send(404, { error: `stub indexer: no route for ${method} ${path}` });
    })();
  });

  return { server, recorded };
}

const isEntrypoint = process.argv[1]?.endsWith("stubIndexer.ts") ?? false;
if (isEntrypoint) {
  const port = Number(process.env.STUB_PORT || "8099");
  const { server } = createStubIndexer();
  server.listen(port, "127.0.0.1", () => {
    process.stdout.write(`stub indexer listening on http://127.0.0.1:${port}\n`);
  });
}
