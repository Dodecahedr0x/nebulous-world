"use client";

import { FIND_ANSWER_OPTIONS, FIND_MAX_QUESTIONS } from "@/lib/constants";
import type { FindQuestion } from "@/lib/types";

// Focus is never implied by colour alone here — every control keeps a visible
// ring, since the answer buttons are the only way through the funnel.
const FOCUS_RING =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cobalt focus-visible:ring-offset-2 focus-visible:ring-offset-ivory";

export function QuestionCard({
  question,
  questionsAsked,
  disabled,
  onAnswer,
  onBack,
  onRestart,
  onShowResults,
  canGoBack,
}: {
  question: FindQuestion;
  questionsAsked: number;
  disabled: boolean;
  onAnswer: (value: "yes" | "no" | "skip") => void;
  onBack: () => void;
  onRestart: () => void;
  onShowResults: () => void;
  canGoBack: boolean;
}) {
  return (
    <div className="card p-6 sm:p-8">
      {/* "up to" is load-bearing: the funnel stops as soon as the posterior
          separates, so promising exactly 8 would be a number we break most
          of the time. */}
      <p className="text-xs uppercase tracking-wide text-slate-steel">
        Question {questionsAsked + 1} of up to {FIND_MAX_QUESTIONS}
      </p>

      <div aria-live="polite" className="mt-3">
        <h2 className="font-display text-heading-sm font-normal leading-snug tracking-tight text-ink">
          {question.prompt}
        </h2>
      </div>

      <div className="mt-6 flex flex-wrap gap-3">
        {FIND_ANSWER_OPTIONS.map((option) => (
          <button
            key={option.value}
            type="button"
            disabled={disabled}
            onClick={() => onAnswer(option.value)}
            className={`${option.value === "skip" ? "btn-secondary" : "btn-primary"} ${FOCUS_RING}`}
          >
            {option.label}
          </button>
        ))}
      </div>

      {/* Available from the first question onward, not just near the end —
          a visitor who has learned enough must always be able to leave. */}
      {canGoBack && (
        <div className="mt-6 flex flex-wrap items-center gap-2 border-t border-hairline pt-4">
          <button
            type="button"
            disabled={disabled}
            onClick={onBack}
            className={`btn-ghost px-3 py-2 text-sm ${FOCUS_RING}`}
          >
            Back
          </button>
          <button
            type="button"
            disabled={disabled}
            onClick={onRestart}
            className={`btn-ghost px-3 py-2 text-sm ${FOCUS_RING}`}
          >
            Start over
          </button>
          <button
            type="button"
            disabled={disabled}
            onClick={onShowResults}
            className={`btn-secondary ml-auto px-4 py-2 text-sm ${FOCUS_RING}`}
          >
            Show me results
          </button>
        </div>
      )}
    </div>
  );
}
