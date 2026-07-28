"use client";

import { useEffect } from "react";
import { FIND_ANSWER_OPTIONS, FIND_MAX_QUESTIONS } from "@/lib/constants";
import type { FindQuestion } from "@/lib/types";
import { cn } from "@/lib/utils";

// Focus is never implied by colour alone here — every control keeps a visible
// ring, since the answer buttons are the only way through the funnel.
const FOCUS_RING =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cobalt focus-visible:ring-offset-2 focus-visible:ring-offset-ivory";

/** Concentric radii: the answer tiles sit in a `p-2` well inside the card, so
    outer (20) = inner (12) + padding (8). Matching radii on nested surfaces is
    the single most common reason a card reads as "off". */
const CARD_RADIUS = "rounded-[20px]";
const TILE_RADIUS = "rounded-[12px]";

export function QuestionCard({
  question,
  questionsAsked,
  candidateCount,
  disabled,
  onAnswer,
  onBack,
  onRestart,
  onShowResults,
  canGoBack,
}: {
  question: FindQuestion;
  questionsAsked: number;
  candidateCount: number;
  disabled: boolean;
  onAnswer: (value: "yes" | "no" | "skip") => void;
  onBack: () => void;
  onRestart: () => void;
  onShowResults: () => void;
  canGoBack: boolean;
}) {
  // Answering by keyboard is what makes this feel like a game rather than a
  // form — the rhythm only appears once you stop aiming at buttons. Ignored
  // while a request is in flight so a fast double-press cannot answer twice.
  useEffect(() => {
    if (disabled) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      const match = FIND_ANSWER_OPTIONS.find((o) => o.key === event.key.toLowerCase());
      if (!match) return;
      event.preventDefault();
      onAnswer(match.value);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [disabled, onAnswer]);

  return (
    <div className={cn(CARD_RADIUS, "border border-hairline bg-ivory p-6 shadow-rest sm:p-8")}>
      <div className="flex items-start justify-between gap-4">
        {/* Pips, not a progress bar: a bar implies a fixed length, and the
            funnel stops as soon as the posterior separates. Pips read as turns
            taken, which is true however early it ends. */}
        <div
          className="mt-1.5 flex items-center gap-1.5"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={FIND_MAX_QUESTIONS}
          aria-valuenow={questionsAsked}
          aria-label={`Question ${questionsAsked + 1} of up to ${FIND_MAX_QUESTIONS}`}
        >
          {Array.from({ length: FIND_MAX_QUESTIONS }, (_, i) => (
            <span
              key={i}
              className={cn(
                "h-2 rounded-pill transition-[width,background-color] duration-300 ease-out",
                i < questionsAsked && "w-7 bg-cobalt",
                i === questionsAsked && "w-7 bg-cobalt/35",
                i > questionsAsked && "w-2 bg-powder",
              )}
            />
          ))}
        </div>

        {/* The stakes of the whole game, and the number was already sitting
            unused in the API response — every turn should visibly cost the
            field something. Sized like a scoreboard rather than a footnote,
            because watching it fall IS the game. tabular-nums so a three-digit
            count collapsing to two does not shift the row. */}
        <p className="shrink-0 text-right leading-none" aria-live="polite">
          <span className="block font-mono text-heading-sm font-medium tabular-nums text-ink">
            {candidateCount}
          </span>
          <span className="mt-1 block text-caption text-slate-steel">still in play</span>
        </p>
      </div>

      {/* Keyed on the prompt so every new question replays the stagger — the
          question lands first, the choices follow. */}
      <div key={question.prompt}>
        <h2
          className="animate-fade-in mt-6 text-balance font-display text-heading-sm font-semibold leading-tight tracking-tight text-ink sm:text-heading-lg"
          aria-live="polite"
        >
          {question.prompt}
        </h2>

        <div
          className={cn(TILE_RADIUS, "animate-fade-in mt-6 grid grid-cols-3 gap-2")}
          style={{ animationDelay: "100ms", animationFillMode: "backwards" }}
        >
          {FIND_ANSWER_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              disabled={disabled}
              onClick={() => onAnswer(option.value)}
              className={cn(
                TILE_RADIUS,
                FOCUS_RING,
                // min-h-[88px] clears the 44px touch target with room to spare —
                // these are the page's primary action, not a dense toolbar.
                "group flex min-h-[88px] flex-col items-center justify-center gap-1.5 border px-3 py-4",
                "text-sm font-medium transition-[background-color,border-color,box-shadow,scale] duration-150 ease-out",
                "active:scale-[0.96] disabled:cursor-not-allowed disabled:opacity-50 disabled:active:scale-100",
                option.value === "skip"
                  ? "border-hairline bg-cream text-slate hover:border-powder hover:text-ink hover:shadow-hover"
                  : "border-hairline bg-cream text-ink hover:border-cobalt/50 hover:bg-indigo-soft hover:shadow-hover",
              )}
            >
              <span className="text-body font-semibold">{option.label}</span>
              <kbd
                aria-hidden
                className="rounded-pill border border-hairline bg-ivory px-1.5 py-0.5 font-mono text-[10px] uppercase text-slate-steel transition-colors duration-150 group-hover:text-slate"
              >
                {option.key}
              </kbd>
            </button>
          ))}
        </div>
      </div>

      {/* Available from the first question onward, not just near the end —
          a visitor who has learned enough must always be able to leave. */}
      {canGoBack && (
        <div className="mt-6 flex flex-wrap items-center gap-1 border-t border-hairline pt-4">
          <button
            type="button"
            disabled={disabled}
            onClick={onBack}
            className={cn(
              "btn-ghost min-h-[40px] px-3 py-2 text-sm transition-[color,background-color,scale] active:scale-[0.96]",
              FOCUS_RING,
            )}
          >
            Back
          </button>
          <button
            type="button"
            disabled={disabled}
            onClick={onRestart}
            className={cn(
              "btn-ghost min-h-[40px] px-3 py-2 text-sm transition-[color,background-color,scale] active:scale-[0.96]",
              FOCUS_RING,
            )}
          >
            Start over
          </button>
          <button
            type="button"
            disabled={disabled}
            onClick={onShowResults}
            className={cn(
              "btn-secondary ml-auto min-h-[40px] px-4 py-2 text-sm transition-[color,background-color,border-color,box-shadow,scale] active:scale-[0.96]",
              FOCUS_RING,
            )}
          >
            Show me results
          </button>
        </div>
      )}
    </div>
  );
}
