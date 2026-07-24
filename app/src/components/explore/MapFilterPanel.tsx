"use client";

import { useState } from "react";
import { cn } from "@/lib/utils";
import { useMountTransition } from "@/hooks/useMountTransition";
import { RangeRow, type RangeFilters } from "@/components/discover/FilterPanel";

// Kept in sync with the panel's exit transition — see FilterPanel's own
// identical constant.
const EXIT_MS = 150;

/** Only these 3 of Discover's 4 range pairs apply to the maps — see MapRangeFilters in lib/indexerClient.ts (no "tags stake" there). */
const MAP_RANGE_KEYS: (keyof RangeFilters)[] = [
  "appStakeMin",
  "appStakeMax",
  "tagsCountMin",
  "tagsCountMax",
  "pageviewsMin",
  "pageviewsMax",
];

/** Number of active range filters, for the collapsed toggle's badge count. */
export function countActiveMapFilters(ranges: RangeFilters): number {
  return MAP_RANGE_KEYS.filter((key) => ranges[key] !== "").length;
}

interface Props {
  ranges: RangeFilters;
  onRangeChange: (key: keyof RangeFilters, value: string) => void;
  onClear: () => void;
}

/**
 * A collapsible "advanced search" for the Apps/Group maps — min/max app
 * stake, min/max tag count, min/max pageviews, restricting which apps
 * appear as nodes. Reuses Discover's `RangeRow`/`RangeFilters` and matches
 * its `FilterPanel` toggle button styling (icon, badge, the same
 * merged-corner button/panel treatment) for visual consistency, and anchors
 * the panel as an absolutely-positioned dropdown rather than an inline
 * expansion — this section sits above the map, so pushing it down every
 * time the panel opens/closes would shift (and visually jolt) the map on
 * every toggle. Starts collapsed since most visits to the map don't need
 * it, unlike the tag filter above it which is common enough to always show.
 */
export function MapFilterPanel({ ranges, onRangeChange, onClear }: Props) {
  const [isOpen, setIsOpen] = useState(false);
  const { rendered: panelRendered, visible: panelVisible } = useMountTransition(isOpen, EXIT_MS);

  const activeCount = countActiveMapFilters(ranges);

  return (
    <div className="relative inline-block">
      <button
        type="button"
        onClick={() => setIsOpen((v) => !v)}
        className={cn(
          "btn-secondary gap-2 rounded-pill text-xs",
          panelRendered && "rounded-b-none border-b-0",
        )}
        aria-expanded={isOpen}
        aria-controls="map-filter-panel"
      >
        <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 4h18M6 12h12M10 20h4" />
        </svg>
        Advanced search
        {activeCount > 0 && (
          <span className="grid h-5 min-w-5 place-items-center rounded-full bg-cobalt px-1 text-[11px] font-semibold text-cream">
            {activeCount}
          </span>
        )}
      </button>

      {panelRendered && (
        <div
          id="map-filter-panel"
          className={cn(
            "card absolute left-0 top-full z-20 grid w-[min(36rem,calc(100vw-2rem))] origin-top-left gap-4 rounded-tl-none p-4 transition-opacity duration-150 motion-safe:transition-[opacity,transform] sm:grid-cols-3",
            panelVisible ? "opacity-100 motion-safe:scale-100" : "opacity-0 motion-safe:scale-95",
          )}
        >
          <RangeRow label="App stake" minKey="appStakeMin" maxKey="appStakeMax" ranges={ranges} onRangeChange={onRangeChange} />
          <RangeRow label="Number of tags" minKey="tagsCountMin" maxKey="tagsCountMax" ranges={ranges} onRangeChange={onRangeChange} />
          <RangeRow label="Pageviews" minKey="pageviewsMin" maxKey="pageviewsMax" ranges={ranges} onRangeChange={onRangeChange} />
          {activeCount > 0 && (
            <button className="text-left text-xs text-cobalt hover:underline sm:col-span-3" onClick={onClear}>
              Clear advanced search
            </button>
          )}
        </div>
      )}
    </div>
  );
}
