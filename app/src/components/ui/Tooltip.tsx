"use client";

import { useId, useState } from "react";

/**
 * A small "?" trigger that reveals a hairline-bordered popover on hover/focus
 * — same `bg-cream` + `shadow-hover` elevation as any other popover (see
 * DESIGN.md's "Surface Raised" row), not a colored/dark bubble.
 */
export function Tooltip({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();

  return (
    <span className="relative inline-flex">
      <button
        type="button"
        aria-describedby={id}
        aria-label="More info"
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        className="inline-flex h-3.5 w-3.5 items-center justify-center rounded-full border border-hairline text-[9px] font-medium leading-none text-slate-steel hover:border-cobalt hover:text-cobalt"
      >
        ?
      </button>
      {open && (
        <span
          id={id}
          role="tooltip"
          className="absolute bottom-full left-1/2 z-10 mb-1.5 w-56 -translate-x-1/2 rounded-md border border-hairline bg-cream p-2 text-xs text-slate shadow-hover"
        >
          {text}
        </span>
      )}
    </span>
  );
}
