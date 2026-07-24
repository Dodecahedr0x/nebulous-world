"use client";

import { useState } from "react";
import type { SearchResult } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useMountTransition } from "@/hooks/useMountTransition";
import { TagAutocomplete } from "@/components/explore/TagAutocomplete";

// Kept in sync with the panel's `duration-150` exit transition — it stays
// mounted this long after closing so the fade/scale-out can play instead of
// the panel disappearing mid-animation.
const EXIT_MS = 150;

export interface RangeFilters {
  appStakeMin: string;
  appStakeMax: string;
  tagsStakeMin: string;
  tagsStakeMax: string;
  tagsCountMin: string;
  tagsCountMax: string;
  pageviewsMin: string;
  pageviewsMax: string;
}

export const EMPTY_RANGE_FILTERS: RangeFilters = {
  appStakeMin: "",
  appStakeMax: "",
  tagsStakeMin: "",
  tagsStakeMax: "",
  tagsCountMin: "",
  tagsCountMax: "",
  pageviewsMin: "",
  pageviewsMax: "",
};

interface Props {
  facets: SearchResult["facets"];
  selectedTags: string[];
  onToggleTag: (slug: string) => void;
  ranges: RangeFilters;
  onRangeChange: (key: keyof RangeFilters, value: string) => void;
  fuzzy: string;
  onFuzzyChange: (value: string) => void;
  onClear: () => void;
}

/** Exported for reuse by the maps' advanced-search panel (see explore/MapFilterPanel.tsx) — same min/max number-input pair UI. */
export function RangeRow({
  label,
  minKey,
  maxKey,
  ranges,
  onRangeChange,
}: {
  label: string;
  minKey: keyof RangeFilters;
  maxKey: keyof RangeFilters;
  ranges: RangeFilters;
  onRangeChange: (key: keyof RangeFilters, value: string) => void;
}) {
  return (
    <div>
      <span className="mb-1 block text-xs font-medium text-slate">{label}</span>
      <div className="flex items-center gap-2">
        <input
          type="number"
          min={0}
          step={1}
          inputMode="numeric"
          className="input px-2 py-1.5 text-sm"
          placeholder="Min"
          value={ranges[minKey]}
          onChange={(e) => onRangeChange(minKey, e.target.value)}
          aria-label={`${label} minimum`}
        />
        <span className="text-slate-steel">–</span>
        <input
          type="number"
          min={0}
          step={1}
          inputMode="numeric"
          className="input px-2 py-1.5 text-sm"
          placeholder="Max"
          value={ranges[maxKey]}
          onChange={(e) => onRangeChange(maxKey, e.target.value)}
          aria-label={`${label} maximum`}
        />
      </div>
    </div>
  );
}

/** Number of active filters, for the collapsed toggle's badge count. */
export function countActiveFilters(
  selectedTags: string[],
  ranges: RangeFilters,
  fuzzy: string,
): number {
  const rangeCount = Object.values(ranges).filter((v) => v !== "").length;
  return selectedTags.length + rangeCount + (fuzzy.trim() ? 1 : 0);
}

/**
 * The Discover filters: a floating, collapsible panel anchored to the right
 * edge of the viewport (starts collapsed). Filters split cleanly along the
 * product's own data model — tags (onchain, stake-weighted) and OpenGraph
 * text (offchain) — there is no separate "category" taxonomy here.
 */
export function FilterPanel({
  facets,
  selectedTags,
  onToggleTag,
  ranges,
  onRangeChange,
  fuzzy,
  onFuzzyChange,
  onClear,
}: Props) {
  const [isOpen, setIsOpen] = useState(false);
  const { rendered: panelRendered, visible: panelVisible } = useMountTransition(isOpen, EXIT_MS);

  const activeCount = countActiveFilters(selectedTags, ranges, fuzzy);
  const hasFilters = activeCount > 0;

  return (
    <div className="fixed right-4 top-24 z-40 flex flex-col items-end">
      <button
        type="button"
        onClick={() => setIsOpen((v) => !v)}
        className={cn(
          "btn-secondary gap-2 rounded-pill",
          panelRendered && "rounded-b-none border-b-0",
        )}
        aria-expanded={isOpen}
        aria-controls="discover-filter-panel"
      >
        <svg
          className="h-4 w-4"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M3 4h18M6 12h12M10 20h4"
          />
        </svg>
        Filters
        {hasFilters && (
          <span className="grid h-5 min-w-5 place-items-center rounded-full bg-cobalt px-1 text-[11px] font-semibold text-cream">
            {activeCount}
          </span>
        )}
      </button>

      {panelRendered && (
        <div
          id="discover-filter-panel"
          className={cn(
            "card mt-0 max-h-[75vh] w-[calc(100vw-2rem)] origin-top-right space-y-5 overflow-y-auto rounded-tr-none p-4 transition-opacity duration-150 motion-safe:transition-[opacity,transform] sm:w-72",
            panelVisible ? "opacity-100 motion-safe:scale-100" : "opacity-0 motion-safe:scale-95",
          )}
        >
          <div className="flex items-center justify-between">
            <span className="text-sm font-semibold text-ink">Filters</span>
            {hasFilters && (
              <button
                className="text-xs text-cobalt hover:underline"
                onClick={onClear}
              >
                Clear
              </button>
            )}
          </div>

          <div>
            <h3 className="mb-2 text-caption font-semibold uppercase tracking-[0.077em] text-slate-steel">
              Tags
            </h3>
            <TagAutocomplete
              options={facets.tags.map((t) => ({ id: t.slug, name: t.name, meta: String(t.count) }))}
              excludeIds={selectedTags}
              onSelect={onToggleTag}
              placeholder="Search tags…"
              ariaLabel="Search tags"
            />
            {selectedTags.length > 0 && (
              <div className="mt-2 flex max-h-48 flex-wrap gap-1.5 overflow-y-auto">
                {selectedTags.map((slug) => (
                  <button key={slug} onClick={() => onToggleTag(slug)} className="chip chip-active">
                    #{facets.tags.find((t) => t.slug === slug)?.name ?? slug}
                    <span aria-hidden="true">✕</span>
                  </button>
                ))}
              </div>
            )}
          </div>

          <div>
            <h3 className="mb-2 text-caption font-semibold uppercase tracking-[0.077em] text-slate-steel">
              Description search
            </h3>
            <input
              className="input px-2 py-1.5 text-sm"
              placeholder="Fuzzy match name, tagline, description…"
              value={fuzzy}
              onChange={(e) => onFuzzyChange(e.target.value)}
              aria-label="Fuzzy search app text"
            />
          </div>

          <div className="space-y-4">
            <h3 className="text-caption font-semibold uppercase tracking-[0.077em] text-slate-steel">
              Ranges
            </h3>
            <RangeRow
              label="App stake"
              minKey="appStakeMin"
              maxKey="appStakeMax"
              ranges={ranges}
              onRangeChange={onRangeChange}
            />
            <RangeRow
              label="Tags stake"
              minKey="tagsStakeMin"
              maxKey="tagsStakeMax"
              ranges={ranges}
              onRangeChange={onRangeChange}
            />
            <RangeRow
              label="Number of tags"
              minKey="tagsCountMin"
              maxKey="tagsCountMax"
              ranges={ranges}
              onRangeChange={onRangeChange}
            />
            <RangeRow
              label="Pageviews"
              minKey="pageviewsMin"
              maxKey="pageviewsMax"
              ranges={ranges}
              onRangeChange={onRangeChange}
            />
          </div>
        </div>
      )}
    </div>
  );
}
