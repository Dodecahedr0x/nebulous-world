"use client";

import { useState } from "react";
import { AppCard } from "@/components/AppCard";
import { FIND_ANSWER_OPTIONS } from "@/lib/constants";
import type { FindAnswer, FindShortlistEntry } from "@/lib/types";

const FOCUS_RING =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cobalt focus-visible:ring-offset-2 focus-visible:ring-offset-ivory";

function answerLabel(value: FindAnswer["value"]): string {
  return FIND_ANSWER_OPTIONS.find((o) => o.value === value)?.label ?? value;
}

/** What the visitor actually told us — not what the engine concluded. The
    response carries only a confidence number, no per-app explanation, so
    inventing a "we picked this because…" line would be fiction. "Don't care"
    answers are omitted because they perform no scoring update at all: listing
    them would imply they shaped the result. */
function AnswerSummary({ answers }: { answers: FindAnswer[] }) {
  const stated = answers.filter((a) => a.value !== "skip");
  if (stated.length === 0) return null;

  return (
    <div>
      <h3 className="text-xs uppercase tracking-wide text-slate-steel">What you told us</h3>
      <ul className="mt-2 flex flex-wrap gap-2">
        {stated.map((a) => (
          <li key={`${a.facet.kind}:${a.facet.value}`} className="chip">
            <span className="text-ink">{a.facet.value}</span>
            <span className="text-slate-steel">·</span>
            <span>{answerLabel(a.value)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function FindResults({
  shortlist,
  answers,
  onConfirm,
  onReject,
  onRestart,
  busy,
}: {
  shortlist: FindShortlistEntry[];
  answers: FindAnswer[];
  onConfirm: (appId: string) => void;
  onReject: (appId: string) => void;
  onRestart: () => void;
  busy: boolean;
}) {
  // "Not quite" advances this cursor to the next-best suggestion rather than
  // ending the session (A9): a rejection is the most informative moment in
  // the funnel, and the visitor still has not got what they came for.
  const [cursor, setCursor] = useState(0);
  const [confirmed, setConfirmed] = useState(false);

  if (shortlist.length === 0) {
    return (
      <div className="card p-6 text-center sm:p-8">
        <h2 className="font-display text-heading-sm font-normal text-ink">
          Nothing to suggest yet
        </h2>
        <p className="mt-2 text-body text-slate">
          We could not narrow this down. Starting over with different answers usually helps.
        </p>
        <button type="button" onClick={onRestart} className={`btn-primary mt-5 ${FOCUS_RING}`}>
          Start over
        </button>
      </div>
    );
  }

  const top = shortlist[cursor];
  const rest = shortlist.filter((_, i) => i !== cursor);

  if (confirmed && top) {
    return (
      <div className="space-y-6">
        <div className="card p-6 sm:p-8">
          <h2 className="font-display text-heading-sm font-normal text-ink">
            Glad we found it
          </h2>
          <p className="mt-2 text-body text-slate">
            {top.app.name} it is. You can vote or stake on it right from its card.
          </p>
          <button
            type="button"
            onClick={onRestart}
            className={`btn-secondary mt-5 ${FOCUS_RING}`}
          >
            Find another app
          </button>
        </div>
        <AppCard app={top.app} />
        <AnswerSummary answers={answers} />
      </div>
    );
  }

  // Every suggestion was rejected. The funnel is out of candidates rather
  // than out of questions, so start over is the only honest move left.
  if (!top) {
    return (
      <div className="card p-6 text-center sm:p-8">
        <h2 className="font-display text-heading-sm font-normal text-ink">
          That is everything we had
        </h2>
        <p className="mt-2 text-body text-slate">
          None of those fit. Different answers will steer us somewhere else.
        </p>
        <button type="button" onClick={onRestart} className={`btn-primary mt-5 ${FOCUS_RING}`}>
          Start over
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Keyed on the app so "Not quite" REPLAYS the reveal for the next
          candidate rather than swapping text under a static frame. The beat is
          the point: each rejection should feel like the game answering again. */}
      <div key={top.app.id} className="space-y-6">
        <div className="animate-fade-in rounded-[20px] border border-hairline bg-ivory p-6 shadow-rest sm:p-8">
          <p className="text-caption uppercase tracking-wide text-slate-steel">Our suggestion</p>
          <h2 className="mt-1 text-balance font-display text-heading-sm font-semibold leading-tight tracking-tight text-ink">
            Is this the one?
          </h2>
          {/* Confidence is spelled out in words as well as drawn as a bar —
              the bar alone would make length the sole signal. */}
          <p className="mt-2 text-sm text-slate">
            <span className="font-mono tabular-nums text-ink">
              {Math.round(top.confidence * 100)}%
            </span>{" "}
            confidence, based on your answers.
          </p>
          <div
            role="presentation"
            className="mt-2 h-1.5 w-full overflow-hidden rounded-pill bg-mist"
          >
            <div
              className="h-full rounded-pill bg-cobalt transition-[width] duration-500 ease-out"
              style={{ width: `${Math.round(top.confidence * 100)}%` }}
            />
          </div>

          <div className="mt-5 flex flex-wrap gap-3">
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                setConfirmed(true);
                onConfirm(top.app.id);
              }}
              className={`btn-primary transition-[color,background-color,box-shadow,scale] active:scale-[0.96] disabled:active:scale-100 ${FOCUS_RING}`}
            >
              Yes, that is it
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                onReject(top.app.id);
                setCursor((c) => c + 1);
              }}
              className={`btn-secondary transition-[color,background-color,border-color,box-shadow,scale] active:scale-[0.96] disabled:active:scale-100 ${FOCUS_RING}`}
            >
              Not quite
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={onRestart}
              className={`btn-ghost ml-auto min-h-[40px] px-4 py-2 text-sm transition-[color,background-color,scale] active:scale-[0.96] ${FOCUS_RING}`}
            >
              Start over
            </button>
          </div>
        </div>

        {/* One beat behind the header, so the answer arrives rather than
            appearing already there. backwards fill keeps it hidden through
            the delay instead of flashing at full opacity first. */}
        <div
          className="animate-fade-in"
          style={{ animationDelay: "110ms", animationFillMode: "backwards" }}
        >
          <AppCard app={top.app} />
        </div>
      </div>

      {rest.length > 0 && (
        <div>
          <h3 className="text-xs uppercase tracking-wide text-slate-steel">
            Others that came close
          </h3>
          <div className="mt-3 grid gap-4 sm:grid-cols-2">
            {rest.map((entry) => (
              <AppCard key={entry.app.id} app={entry.app} />
            ))}
          </div>
        </div>
      )}

      <AnswerSummary answers={answers} />
    </div>
  );
}
