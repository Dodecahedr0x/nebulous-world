"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { cn } from "@/lib/utils";
import type { TagGraph } from "@/lib/indexerClient";
type TagGraphNode = TagGraph["nodes"][number];
import { AppMap } from "./AppMap";
import { TagMap } from "./TagMap";
import { GroupMap } from "./GroupMap";
import { RelatedApps, type MapSelection } from "./RelatedApps";
import { TagAutocomplete } from "./TagAutocomplete";
import { MapFilterPanel } from "./MapFilterPanel";
import type { MapNode } from "./ForceMap";
import { EMPTY_RANGE_FILTERS, type RangeFilters } from "@/components/discover/FilterPanel";
import type { MapRangeFilters } from "@/lib/indexerClient";

type TabKey = "apps" | "tags" | "group";

const TABS: { key: TabKey; label: string; description: string }[] = [
  {
    key: "apps",
    label: "Apps",
    description:
      "Apps cluster together when they're tagged alike — a quick way to find something close to an app you already use. Combine tags below to narrow the map down.",
  },
  {
    key: "tags",
    label: "Tags",
    description:
      "Bigger circles have more stake behind them. Tags placed close together tend to show up on the same apps — a way to browse by theme instead of by keyword.",
  },
  {
    key: "group",
    label: "Group",
    description:
      "Apps nested by tag, from the most common tag down to the most specific. Outer circles are broad themes, inner circles narrow them down, and the smallest filled circles are individual apps. Click an app to zoom in, or a tag (here or below) to filter down to it — drag to pan, scroll to zoom.",
  },
];

// Cap the tag-filter picker to the most-used tags — same reasoning as the
// Discover page's facet list: showing every tag ever suggested (including
// barely-used ones) would make the picker itself unusable.
const MAX_FILTER_TAGS = 30;

// Only these 3 of Discover's 4 range pairs apply to the maps' advanced
// search (no "tags stake") — see MapFilterPanel's own doc comment. Typed
// against both RangeFilters and MapRangeFilters so `toMapRangeFilters`
// below can index either with the same key.
const RANGE_KEYS: (keyof RangeFilters & keyof MapRangeFilters)[] = [
  "appStakeMin",
  "appStakeMax",
  "tagsCountMin",
  "tagsCountMax",
  "pageviewsMin",
  "pageviewsMax",
];

function toMapRangeFilters(ranges: RangeFilters): MapRangeFilters {
  const out: MapRangeFilters = {};
  for (const key of RANGE_KEYS) {
    if (ranges[key]) out[key] = ranges[key];
  }
  return out;
}

/**
 * Everything the Explore page's constellation maps need, in one panel: a
 * tab bar switching between the app map and the tag map (only the active
 * one is mounted, so only one force simulation ever runs at a time), a
 * tag-combination filter for the app map, and a selected-node "connected
 * apps" panel shared by both tabs. Selecting a node on one map and
 * switching tabs clears the selection, since a tag selection and an app
 * selection aren't the same kind of thing to carry across — but the tag
 * filter itself persists across tabs, since it's a standing preference,
 * not a one-off selection.
 *
 * Both the active tab (`?tab=`) and the tag filter (`?tags=`, repeatable)
 * live in the URL rather than plain local state, so this view is
 * deep-linkable — see AppCard's tag chips, which land on `?view=map&tab=
 * group&tags=<slug>` (the `view` param is RankingsTabs' own, one level up)
 * to open straight into "every app tagged X," filtered, with zero clicks.
 */
export function ExploreMaps() {
  const router = useRouter();
  const params = useSearchParams();
  const rawTab = params.get("tab");
  const tab: TabKey = rawTab === "tags" || rawTab === "group" ? rawTab : "apps";
  const selectedTags = useMemo(() => params.getAll("tags"), [params]);
  const [ranges, setRanges] = useState<RangeFilters>(() => {
    const next = { ...EMPTY_RANGE_FILTERS };
    for (const key of RANGE_KEYS) next[key] = params.get(key) ?? "";
    return next;
  });

  const [selection, setSelection] = useState<MapSelection | null>(null);
  const [availableTags, setAvailableTags] = useState<TagGraphNode[]>([]);
  const panelRef = useRef<HTMLElement>(null);
  const [isFullscreen, setIsFullscreen] = useState(false);

  // Tracks the native Fullscreen API rather than driving a boolean directly
  // off the toggle button — the browser can also exit fullscreen on its own
  // (Esc key, OS gesture), and this is the only way to stay in sync with that.
  useEffect(() => {
    function onChange() {
      setIsFullscreen(document.fullscreenElement === panelRef.current);
    }
    document.addEventListener("fullscreenchange", onChange);
    return () => document.removeEventListener("fullscreenchange", onChange);
  }, []);

  function toggleFullscreen() {
    if (document.fullscreenElement) {
      document.exitFullscreen();
    } else {
      panelRef.current?.requestFullscreen();
    }
  }
  // Drives the Tags tab's "typing a tag selects it on the map" behavior —
  // see ForceMap's `selectRequest` doc comment for why this needs to be a
  // fresh object every time rather than just the tag's id. Seeded from the
  // URL on first render (lazy initializer, so it only ever runs once) when
  // a tag chip elsewhere in the app deep-links straight to `?tab=tags&
  // tags=<slug>` — see TagChip and ForceMap's `pendingSelectRequestRef` for
  // how that reaches the map's own force simulation even though it's still
  // loading at this point. Only the first selected tag applies: this tab's
  // interaction model is single-select (unlike Apps/Group's multi-tag
  // filter, which already reads every `tags=` value via `selectedTags`
  // above).
  const [tagSelectRequest, setTagSelectRequest] = useState<{ id: string } | null>(() =>
    tab === "tags" && selectedTags[0] ? { id: selectedTags[0] } : null,
  );

  useEffect(() => {
    fetch("/api/tags/graph")
      .then((r) => r.json())
      .then((json) => {
        if (!json.ok) return;
        const tags: TagGraphNode[] = json.data.nodes ?? [];
        const mostUsed = [...tags].sort((a, b) => b.appCount - a.appCount).slice(0, MAX_FILTER_TAGS);
        setAvailableTags(mostUsed.sort((a, b) => a.name.localeCompare(b.name)));
      })
      .catch(() => {});
  }, []);

  // A stale request would otherwise re-fire on remount (each tab's map
  // fully remounts via the `key={tab}` below) and silently re-select
  // whatever was last picked — selection doesn't persist across tabs
  // anywhere else in this component either, so this shouldn't be the one
  // exception. Runs only on an ACTUAL tab change: compares against the
  // previous tab (`prevTabRef`, seeded to the mount-time tab) rather than a
  // fires-once flag, because React's Strict Mode double-invokes every
  // effect once in development (mount → simulated cleanup → mount again) —
  // a flag that just flips `false` after its first run gets flipped by
  // that first synthetic invocation and reads as "already past the first
  // render" on the second one, clearing `tagSelectRequest`'s URL-seeded
  // value before the map ever gets a chance to apply it. Comparing against
  // the previous tab is idempotent across that double-invoke instead: both
  // synthetic runs see the same (unchanged) tab and no-op.
  const prevTabRef = useRef(tab);
  useEffect(() => {
    if (prevTabRef.current === tab) return;
    prevTabRef.current = tab;
    setSelection(null);
    setTagSelectRequest(null);
  }, [tab]);

  function pushParams(next: {
    tab?: TabKey;
    tags?: string[];
    ranges?: Partial<Record<keyof RangeFilters, string>>;
  }) {
    const sp = new URLSearchParams(params.toString());
    if (next.tab !== undefined) {
      if (next.tab === "apps") sp.delete("tab");
      else sp.set("tab", next.tab);
    }
    if (next.tags !== undefined) {
      sp.delete("tags");
      for (const t of next.tags) sp.append("tags", t);
    }
    if (next.ranges !== undefined) {
      for (const key of RANGE_KEYS) {
        const value = next.ranges[key];
        if (value === undefined) continue;
        if (value) sp.set(key, value);
        else sp.delete(key);
      }
    }
    const qs = sp.toString();
    router.push(qs ? `/rankings?${qs}` : "/rankings", { scroll: false });
  }

  // One timer per range field, same reasoning as Discover's identical
  // pattern — a single shared timer would let typing in a second field
  // cancel a still-pending navigate for the first.
  const debounceRefs = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  function onRangeChange(key: keyof RangeFilters, value: string) {
    setRanges((prev) => ({ ...prev, [key]: value }));
    clearTimeout(debounceRefs.current[key]);
    debounceRefs.current[key] = setTimeout(() => pushParams({ ranges: { [key]: value } }), 300);
  }

  function clearRanges() {
    setRanges({ ...EMPTY_RANGE_FILTERS });
    const cleared: Partial<Record<keyof RangeFilters, string>> = {};
    for (const key of RANGE_KEYS) cleared[key] = "";
    pushParams({ ranges: cleared });
  }

  function switchTab(next: TabKey) {
    if (next === tab) return;
    pushParams({ tab: next });
  }

  function toggleTagFilter(slug: string) {
    setSelection(null);
    const next = selectedTags.includes(slug)
      ? selectedTags.filter((s) => s !== slug)
      : [...selectedTags, slug];
    pushParams({ tags: next });
  }

  // Tags tab only: picking a tag from the search box selects it on the map
  // exactly as clicking its node would (see TagMap/ForceMap's
  // `selectRequest` prop) — there's no "filter" concept on this tab (every
  // tag is always shown), so typing a tag jumps to it instead of narrowing
  // anything down.
  function selectTagOnMap(slug: string) {
    setTagSelectRequest({ id: slug });
  }

  function handleAppSelect(node: MapNode | null, neighborIds: string[]) {
    if (!node) {
      setSelection(null);
      return;
    }
    setSelection({ kind: "app", label: node.label, slugs: [node.id, ...neighborIds], selectedSlug: node.id });
  }

  function handleTagSelect(node: MapNode | null, neighborIds: string[]) {
    if (!node) {
      setSelection(null);
      return;
    }
    setSelection({ kind: "tag", label: node.label, tagSlugs: [node.id, ...neighborIds] });
  }

  // Unlike AppMap/TagMap (which hand back a raw MapNode + neighbor ids for
  // this component to interpret), GroupMap already knows its own tree
  // structure well enough to build the MapSelection itself.
  function handleGroupSelect(next: MapSelection | null) {
    setSelection(next);
  }

  const active = TABS.find((t) => t.key === tab)!;

  return (
    <div>
      {/* Same light Surface/bg-ivory card treatment as every other panel in
          the app (see .card in globals.css) — this used to be a bespoke
          dark-glass treatment (translucent white overlays, a hand-picked
          abyss → singularity gradient) built for when this was the one dark
          section on an otherwise light-cream page, sitting on an animated
          nebula backdrop. Now that the whole app has been repainted to the
          shared light theme (see DESIGN.md) and the nebula backdrop is gone,
          that bespoke treatment would read as visually out of step with
          everything else — this panel is a card like any other now. */}
      <section ref={panelRef} className="card overflow-hidden p-4 sm:p-6">
        <div className="space-y-4">
          <div className="flex items-center justify-between gap-2">
            <div
              role="tablist"
              aria-label="Explore maps"
              className="inline-flex gap-1 rounded-navitem border border-hairline bg-mist p-1"
            >
              {TABS.map((t) => (
                <button
                  key={t.key}
                  type="button"
                  role="tab"
                  aria-selected={tab === t.key}
                  onClick={() => switchTab(t.key)}
                  className={cn(
                    "rounded-navitem px-4 py-2 text-sm font-medium transition-[color,background-color] duration-150",
                    tab === t.key ? "bg-ivory text-ink" : "text-slate hover:text-ink",
                  )}
                >
                  {t.label}
                </button>
              ))}
            </div>
            <button
              type="button"
              onClick={toggleFullscreen}
              className="rounded border border-hairline px-2.5 py-1.5 text-xs font-medium text-slate transition-colors hover:bg-mist hover:text-ink"
              aria-pressed={isFullscreen}
            >
              {isFullscreen ? "Exit fullscreen" : "Fullscreen"}
            </button>
          </div>

          <p className="text-pretty text-sm text-slate">{active.description}</p>

          {(tab === "apps" || tab === "group") && availableTags.length > 0 && (
            <div>
              <div className="text-caption font-semibold uppercase tracking-wide text-slate">
                Filter by tag{selectedTags.length > 0 ? ` (${selectedTags.length} selected)` : ""}
              </div>
              <div className="mt-1.5 max-w-xs">
                <TagAutocomplete
                  options={availableTags}
                  excludeIds={selectedTags}
                  onSelect={toggleTagFilter}
                  placeholder="Search tags to filter by…"
                  ariaLabel="Search tags to filter the map by"
                />
              </div>
              {/* Selected tags live below the search box, not in it — the
                  box always starts empty (see TagAutocomplete's own doc
                  comment: it's a picker, not a persistent search field), so
                  this is the only place the current filter is visible.
                  Clicking one removes it, same toggle the search box's
                  onSelect uses to add it. */}
              {selectedTags.length > 0 && (
                <div className="mt-1.5 flex flex-wrap gap-1.5">
                  {selectedTags.map((slug) => (
                    <button
                      key={slug}
                      type="button"
                      onClick={() => toggleTagFilter(slug)}
                      className="chip chip-active"
                    >
                      #{availableTags.find((t) => t.id === slug)?.name ?? slug}
                      <span aria-hidden="true">✕</span>
                    </button>
                  ))}
                  <button type="button" onClick={() => pushParams({ tags: [] })} className="chip">
                    Clear filters
                  </button>
                </div>
              )}
              <div className="mt-2">
                <MapFilterPanel ranges={ranges} onRangeChange={onRangeChange} onClear={clearRanges} />
              </div>
            </div>
          )}

          {tab === "tags" && availableTags.length > 0 && (
            <div>
              <div className="text-caption font-semibold uppercase tracking-wide text-slate">
                Jump to a tag
              </div>
              <div className="mt-1.5 max-w-xs">
                <TagAutocomplete
                  options={availableTags}
                  onSelect={selectTagOnMap}
                  placeholder="Search tags…"
                  ariaLabel="Search for a tag to select it on the map"
                />
              </div>
            </div>
          )}

          <div key={tab} className="animate-fade-in-fast">
            {tab === "apps" ? (
              <AppMap onSelect={handleAppSelect} selectedTags={selectedTags} ranges={toMapRangeFilters(ranges)} />
            ) : tab === "tags" ? (
              <TagMap onSelect={handleTagSelect} selectRequest={tagSelectRequest} />
            ) : (
              <GroupMap
                onSelect={handleGroupSelect}
                selectedTags={selectedTags}
                onToggleTag={toggleTagFilter}
                ranges={toMapRangeFilters(ranges)}
              />
            )}
          </div>
        </div>
      </section>

      {selection && <RelatedApps selection={selection} onClear={() => setSelection(null)} />}
    </div>
  );
}
