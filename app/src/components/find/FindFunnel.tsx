"use client";

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Script from "next/script";
import { useToast } from "@/components/ui/Toaster";
import { QuestionCard } from "@/components/find/QuestionCard";
import { FindResults } from "@/components/find/FindResults";
import {
  funnelReducer,
  initialFunnelState,
  canGoBack,
  funnelProgress,
  turnstileSiteKey,
  initialTurnstileTokenState,
  tokenToSend,
  spendToken,
  receiveToken,
  submitOutcome,
  reconcileUrlAnswers,
  parseFunnelAnswers,
  findFunnelHref,
  backNavigation,
  FUNNEL_ANSWERS_PARAM,
  type FunnelState,
} from "@/components/find/funnelState";
import type { FindAnswer, FindNextResult } from "@/lib/types";

// Same bound, and the same reason, as components/app/TrafficBeacon.tsx: if
// Turnstile's callback never fires the confirm must still go out.
const TURNSTILE_TIMEOUT_MS = 4000;

// Deliberately not txClient's apiPost: that module imports @solana/web3.js,
// and /find must work with no wallet and no Solana anything (A4). Same
// {ok, data} envelope, since it is the one every route in lib/api.ts returns.
async function postJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = await res.json();
  if (!json.ok) throw new Error(json.error || `POST ${path} failed`);
  return json.data as T;
}

function seed(initialResult: FindNextResult | null): FunnelState {
  return { ...initialFunnelState, result: initialResult };
}

export function FindFunnel({ initialResult }: { initialResult: FindNextResult | null }) {
  const [state, dispatch] = useReducer(funnelReducer, initialResult, seed);
  const toast = useToast();
  const router = useRouter();
  const answerParam = useSearchParams().get(FUNNEL_ANSWERS_PARAM);

  // The funnel is stateless on the server (A1), so every turn re-sends the
  // whole history. This ref lets a fire-and-forget handler read the state the
  // reducer is about to hold without re-creating the callback per answer.
  const stateRef = useRef<FunnelState>(state);
  stateRef.current = state;

  // How much history the URL carried when this page loaded — see
  // backNavigation. Fixed at first render: every later entry is one we pushed.
  const landingCountRef = useRef<number | null>(null);
  if (landingCountRef.current === null) {
    landingCountRef.current = parseFunnelAnswers(answerParam).length;
  }

  // The confirm endpoint is Turnstile-gated (it writes the funnel's only
  // training signal), so /find renders the same invisible challenge
  // TrafficBeacon does. The challenge is started at mount rather than on
  // click: by the time a visitor has answered their way to "Is this the
  // one?", the token is already sitting in this ref.
  const siteKey = turnstileSiteKey(process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY);
  const widgetRef = useRef<HTMLDivElement>(null);
  const tokenStateRef = useRef(initialTurnstileTokenState);
  const renderedRef = useRef(false);
  // window.turnstile isn't defined synchronously at mount — the Cloudflare
  // script loads on its own schedule, so rendering has to wait for onReady.
  const [turnstileReady, setTurnstileReady] = useState(false);
  // Distinct from state.loading, which covers question fetches: the results
  // view only renders when loading is already false, so it would be a
  // compile-time `false` there and disable nothing.
  const [outcomeBusy, setOutcomeBusy] = useState(false);

  useEffect(() => {
    if (!siteKey || !turnstileReady || renderedRef.current) return;
    if (!widgetRef.current || !window.turnstile) return;
    renderedRef.current = true;
    window.turnstile.render(widgetRef.current, {
      sitekey: siteKey,
      size: "invisible",
      callback: (token: string) => {
        tokenStateRef.current = receiveToken(token);
      },
    });
  }, [siteKey, turnstileReady]);

  // Burns the redeemed token and asks Turnstile for a replacement, so the
  // NEXT outcome in this session can be verified too. Without this, "Not
  // quite" spends the page's only token and the follow-up "Yes" 403s —
  // dropping exactly the corrective sessions the learned term most needs
  // (A61/A62).
  //
  // The cast is deliberate, not sloppy: components/app/TrafficBeacon.tsx
  // already declares `Window.turnstile` globally with only `render` on it,
  // and a second `declare global` with a different shape is a TypeScript
  // duplicate-property error rather than a merge. Widening that shared
  // global is outside this node's scope, so `reset` is narrowed structurally
  // here. The typeof guard degrades to the old single-token behaviour if the
  // script ever ships without it.
  const cycleTurnstileToken = () => {
    tokenStateRef.current = spendToken();
    const widget = window.turnstile as
      | (NonNullable<Window["turnstile"]> & { reset?: (el?: HTMLElement) => void })
      | undefined;
    if (typeof widget?.reset === "function") {
      widget.reset(widgetRef.current ?? undefined);
    }
  };

  const requestNext = useCallback(
    async (answers: FindAnswer[], forceResults: boolean) => {
      try {
        const data = await postJson<FindNextResult>("/api/find/next", {
          answers,
          ...(forceResults ? { forceResults: true } : {}),
        });
        dispatch({ type: "resolved", result: data });
      } catch (err) {
        dispatch({
          type: "failed",
          message: err instanceof Error ? err.message : "Something went wrong.",
        });
      }
    },
    [],
  );

  // The URL is the funnel's durable copy of its own history (A82), so this is
  // the one place mount, refresh, browser Back and browser Forward are all
  // handled: whenever the query string changes, the funnel is made to agree
  // with it. Deliberately keyed on `answerParam` alone — the current state is
  // read through a ref, because re-running on every state change would let the
  // no-question-yet branch retry in a loop after a failed request.
  useEffect(() => {
    reconcileUrlAnswers(
      {
        restore: (answers) => dispatch({ type: "restored", answers }),
        request: (answers) => {
          dispatch({ type: "loading" });
          void requestNext(answers, false);
        },
        replaceUrl: (href) => router.replace(href, { scroll: false }),
      },
      answerParam,
      stateRef.current,
    );
  }, [answerParam, requestNext, router]);

  const handleAnswer = (value: "yes" | "no" | "skip") => {
    const facet = state.result?.question?.facet;
    if (!facet) return;
    const existing = state.answers.findIndex(
      (a) => a.facet.kind === facet.kind && a.facet.value === facet.value,
    );
    const answer: FindAnswer = { facet, value };
    const answers =
      existing === -1
        ? [...state.answers, answer]
        : state.answers.map((a, i) => (i === existing ? answer : a));

    dispatch({ type: "answered", facet, value });
    void requestNext(answers, false);
    // A history entry per question, so the browser's own Back steps back one
    // question instead of leaving /find. The reducer moved first rather than
    // waiting for the URL: this route is force-dynamic, so a push costs a
    // round trip, and the visitor must not sit looking at a live question they
    // have already answered. The effect above then finds the two in agreement.
    router.push(findFunnelHref(answers), { scroll: false });
  };

  const handleBack = () => {
    const step = backNavigation(state.answers.length, landingCountRef.current ?? 0);
    if (step === "none") return;

    const answers = state.answers.slice(0, -1);
    dispatch({ type: "back" });
    void requestNext(answers, false);
    // `pop` walks back over an entry this session pushed, keeping the button
    // and the browser's Back in step. `rewrite` is the shared-link case, where
    // the entry underneath belongs to someone else's browsing.
    if (step === "pop") router.back();
    else router.replace(findFunnelHref(answers), { scroll: false });
  };

  const handleShowResults = () => {
    dispatch({ type: "loading" });
    void requestNext(state.answers, true);
  };

  const handleRestart = () => {
    dispatch({ type: "restart" });
    dispatch({ type: "loading" });
    void requestNext([], false);
    router.push("/find", { scroll: false });
  };

  const recordOutcome = async (appId: string, outcome: "confirmed" | "rejected") => {
    // Covers the token wait too, not just the POST: acquiring a token can
    // take up to TURNSTILE_TIMEOUT_MS, and an unguarded window that long lets
    // repeated "Not quite" clicks fire one outcome write per click.
    setOutcomeBusy(true);
    try {
      // Telemetry, not the visitor's result: a failed write must never
      // discard the shortlist they already earned, so this never dispatches
      // "failed" — it toasts and leaves the results standing.
      await submitOutcome(
        {
          readToken: () => tokenToSend(tokenStateRef.current),
          cycleToken: cycleTurnstileToken,
          post: (body) => postJson("/api/find/confirm", body),
          turnstileEnabled: siteKey !== null,
          timeoutMs: TURNSTILE_TIMEOUT_MS,
        },
        stateRef.current.answers,
        appId,
        outcome,
      );
    } catch {
      toast.error("We could not record that, but your results are unaffected.");
    } finally {
      setOutcomeBusy(false);
    }
  };

  const result = state.result;
  const progress = funnelProgress(state);

  return (
    <div data-testid="find-funnel" className="space-y-6">
      {/* Nothing mounts at all without a site key, so local and simulation
          mode never reach out to Cloudflare. */}
      {siteKey && (
        <>
          <Script
            src="https://challenges.cloudflare.com/turnstile/v0/api.js"
            strategy="afterInteractive"
            onReady={() => setTurnstileReady(true)}
          />
          <div ref={widgetRef} />
        </>
      )}

      <div>
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(progress * 100)}
          aria-label="Funnel progress"
          className="h-1 w-full overflow-hidden rounded-pill bg-mist"
        >
          <div
            className="h-full rounded-pill bg-cobalt transition-[width] duration-300 ease-out"
            style={{ width: `${Math.round(progress * 100)}%` }}
          />
        </div>
      </div>

      {state.error && (
        <div role="alert" className="card border-negative/40 p-4 text-sm text-negative">
          {state.error}{" "}
          <button
            type="button"
            onClick={() => {
              dispatch({ type: "loading" });
              void requestNext(state.answers, false);
            }}
            className="underline underline-offset-2"
          >
            Try again
          </button>
        </div>
      )}

      {state.loading && (
        <p aria-live="polite" className="text-body text-slate-steel">
          Thinking…
        </p>
      )}

      {!state.loading && result?.done && (
        <FindResults
          key={result.shortlist.map((e) => e.app.id).join(",")}
          shortlist={result.shortlist}
          answers={state.answers}
          onConfirm={(appId) => void recordOutcome(appId, "confirmed")}
          onReject={(appId) => void recordOutcome(appId, "rejected")}
          onRestart={handleRestart}
          busy={outcomeBusy}
        />
      )}

      {!state.loading && result && !result.done && result.question && (
        <QuestionCard
          question={result.question}
          questionsAsked={result.questionsAsked}
          disabled={state.loading}
          onAnswer={handleAnswer}
          onBack={handleBack}
          onRestart={handleRestart}
          onShowResults={handleShowResults}
          canGoBack={canGoBack(state)}
        />
      )}
    </div>
  );
}
