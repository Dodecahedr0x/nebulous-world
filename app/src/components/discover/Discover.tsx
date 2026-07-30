"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { AppCard } from "@/components/AppCard";
import {
  EMPTY_RANGE_FILTERS,
  FilterPanel,
  type RangeFilters,
} from "@/components/discover/FilterPanel";
import { CreateAppForm } from "@/components/discover/CreateAppForm";
import { OnboardingBanner } from "@/components/discover/OnboardingBanner";
import { Modal } from "@/components/ui/Modal";
import { PageHeader } from "@/components/PageHeader";
import { SORT_OPTIONS } from "@/lib/constants";
import type { AppDTO, SearchResult } from "@/lib/types";

interface Props {
  initial: SearchResult;
}

const RANGE_KEYS = Object.keys(EMPTY_RANGE_FILTERS) as (keyof RangeFilters)[];
// Keeps page 1's amounts (rank/stake/views) live while the user is at rest
// on it. Paused once they've scrolled into page 2+ (see the effect below) so
// a background refresh never clobbers infinite-scroll-loaded results.
const DISCOVER_POLL_MS = 15000;

/**
 * The Discover experience: a search box, a floating filter panel, a sort
 * control, and an infinite-scrolling results grid. Every filter/sort/query
 * control lives in the URL so results are shareable and the back button
 * works; changing any of them pushes new query params and refetches page 1
 * from /api/apps. Scroll position itself is not part of that shareable
 * state — a fresh page load always starts at page 1, then grows as the user
 * scrolls (see the sentinel ref below).
 */
export function Discover({ initial }: Props) {
  const router = useRouter();
  const params = useSearchParams();

  const [query, setQuery] = useState(params.get("q") ?? "");
  const [fuzzy, setFuzzy] = useState(params.get("fuzzy") ?? "");
  const [ranges, setRanges] = useState<RangeFilters>(() => {
    const next = { ...EMPTY_RANGE_FILTERS };
    for (const key of RANGE_KEYS) next[key] = params.get(key) ?? "";
    return next;
  });
  const [apps, setApps] = useState<AppDTO[]>(initial.apps);
  const [total, setTotal] = useState(initial.total);
  const [facets, setFacets] = useState(initial.facets);
  // How many pages have been loaded into `apps` so far — bumped by loadMore,
  // reset to 1 whenever the filters/sort effect below replaces the list.
  const [loadedPage, setLoadedPage] = useState(1);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  // ?create=1 auto-opens the modal — lets other pages (e.g. the profile's
  // "earn more XP today" panel) deep-link straight into the create flow
  // instead of just landing here and requiring a manual click.
  const [createOpen, setCreateOpen] = useState(() => params.get("create") === "1");
  // Bumped after a successful create to force the fetch effect below to
  // re-run against the current params (new app shows up immediately if it
  // matches the active filters/sort, and the count updates either way) —
  // without touching the URL, which would reset the user's filters.
  const [refreshKey, setRefreshKey] = useState(0);
  // One timer per debounced field — a single shared timer would let typing in
  // a second field (e.g. a range input) cancel a still-pending navigate for
  // the first (e.g. the fuzzy box), silently dropping that field's update.
  const debounceRefs = useRef<Record<string, ReturnType<typeof setTimeout>>>(
    {},
  );
  const debounce = useCallback((key: string, fn: () => void) => {
    clearTimeout(debounceRefs.current[key]);
    debounceRefs.current[key] = setTimeout(fn, 300);
  }, []);

  const selectedTags = useMemo(() => params.getAll("tags"), [params]);
  const sort = params.get("sort") ?? "rank";

  // Build a query string from the current control values — never a "page":
  // results accumulate through infinite scroll, not page navigation, so
  // scroll depth was never part of the shareable URL state.
  const buildParams = useCallback(
    (overrides: Record<string, string | string[] | undefined>) => {
      const next = new URLSearchParams();
      const q = overrides.q !== undefined ? overrides.q : query;
      if (q && typeof q === "string") next.set("q", q);

      const fz = overrides.fuzzy !== undefined ? overrides.fuzzy : fuzzy;
      if (fz && typeof fz === "string") next.set("fuzzy", fz);

      for (const key of RANGE_KEYS) {
        const v = overrides[key] !== undefined ? overrides[key] : ranges[key];
        if (v && typeof v === "string") next.set(key, v);
      }

      const s = overrides.sort !== undefined ? overrides.sort : sort;
      if (s && typeof s === "string" && s !== "rank") next.set("sort", s);

      const tags =
        overrides.tags !== undefined
          ? (overrides.tags as string[])
          : selectedTags;
      for (const t of tags) next.append("tags", t);

      return next;
    },
    [query, fuzzy, ranges, sort, selectedTags],
  );

  const navigate = useCallback(
    (overrides: Record<string, string | string[] | undefined>) => {
      const next = buildParams(overrides);
      router.push(`/?${next.toString()}`, { scroll: false });
    },
    [buildParams, router],
  );

  // Latest `loadedPage` for the background poll below to read without
  // needing to be an effect dependency (a value it must react to without
  // tearing down/restarting its interval every time the user scrolls).
  const loadedPageRef = useRef(loadedPage);
  loadedPageRef.current = loadedPage;

  // Refetch page 1 whenever the URL's filters/sort change — replaces
  // whatever infinite-scroll pages had accumulated so far with a fresh set.
  // Also keeps re-fetching it on an interval afterwards (silently — no
  // `loading` toggle — while the user is still on page 1) so grid amounts
  // don't go stale for someone who lands here and doesn't scroll.
  useEffect(() => {
    let cancelled = false;

    function fetchPage1() {
      return fetch(`/api/apps?${params.toString()}`, { cache: "no-store" })
        .then((r) => r.json())
        .then((json) => {
          if (cancelled || !json.ok) return;
          setApps(json.data.apps);
          setTotal(json.data.total);
          setFacets(json.data.facets);
          setLoadedPage(1);
        });
    }

    setLoading(true);
    fetchPage1().finally(() => {
      if (!cancelled) setLoading(false);
    });

    function pollIfAtRest() {
      if (document.visibilityState === "visible" && loadedPageRef.current === 1) {
        fetchPage1().catch(() => {});
      }
    }
    const interval = setInterval(pollIfAtRest, DISCOVER_POLL_MS);
    document.addEventListener("visibilitychange", pollIfAtRest);
    window.addEventListener("focus", pollIfAtRest);

    return () => {
      cancelled = true;
      clearInterval(interval);
      document.removeEventListener("visibilitychange", pollIfAtRest);
      window.removeEventListener("focus", pollIfAtRest);
    };
  }, [params, refreshKey]);

  // Infinite scroll: fetch the next page (same filters/sort as the current
  // results) and append once the sentinel below the grid enters the
  // viewport — see the sentinelRef callback ref further down.
  const hasMore = apps.length < total;
  const loadingRef = useRef(loading);
  loadingRef.current = loading;
  const loadingMoreRef = useRef(false);
  const loadMore = useCallback(() => {
    if (loadingMoreRef.current || loadingRef.current || !hasMore) return;
    loadingMoreRef.current = true;
    setLoadingMore(true);
    const next = new URLSearchParams(params.toString());
    next.set("page", String(loadedPage + 1));
    fetch(`/api/apps?${next.toString()}`, { cache: "no-store" })
      .then((r) => r.json())
      .then((json) => {
        if (!json.ok) return;
        setApps((prev) => [...prev, ...json.data.apps]);
        setLoadedPage((p) => p + 1);
      })
      .finally(() => {
        loadingMoreRef.current = false;
        setLoadingMore(false);
      });
  }, [params, loadedPage, hasMore]);

  // A callback ref (not a plain ref + effect) so the observer is correctly
  // torn down/rebuilt as the sentinel div itself mounts and unmounts (it
  // only renders while `hasMore`) and whenever `loadMore` gets a fresh
  // closure (new filters/page).
  const observerRef = useRef<IntersectionObserver | null>(null);
  const sentinelRef = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect();
      if (!node) return;
      observerRef.current = new IntersectionObserver(
        (entries) => {
          if (entries[0]?.isIntersecting) loadMore();
        },
        { rootMargin: "600px" },
      );
      observerRef.current.observe(node);
    },
    [loadMore],
  );

  // Debounced text search.
  const onQueryChange = (value: string) => {
    setQuery(value);
    debounce("q", () => navigate({ q: value }));
  };

  const onFuzzyChange = (value: string) => {
    setFuzzy(value);
    debounce("fuzzy", () => navigate({ fuzzy: value }));
  };

  const onRangeChange = (key: keyof RangeFilters, value: string) => {
    setRanges((prev) => ({ ...prev, [key]: value }));
    debounce(key, () => navigate({ [key]: value }));
  };

  const toggleTag = (slug: string) => {
    const next = selectedTags.includes(slug)
      ? selectedTags.filter((t) => t !== slug)
      : [...selectedTags, slug];
    navigate({ tags: next });
  };

  const clearFilters = () => {
    setFuzzy("");
    setRanges({ ...EMPTY_RANGE_FILTERS });
    const overrides: Record<string, string | string[] | undefined> = {
      tags: [],
      fuzzy: "",
      q: query,
    };
    for (const key of RANGE_KEYS) overrides[key] = "";
    navigate(overrides);
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Browse apps"
        description="Ranked by the crowd — token-weighted votes, tag stake, and real traffic."
      />

      <OnboardingBanner />

      <div className="flex flex-col gap-3 sm:flex-row">
        <div className="relative flex-1">
          <input
            className="input pl-10"
            placeholder="Search apps, tags, descriptions…"
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            aria-label="Search apps"
          />
          <svg
            className="pointer-events-none absolute left-3 top-2.5 h-5 w-5 text-slate-steel"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M21 21l-4.35-4.35M11 18a7 7 0 100-14 7 7 0 000 14z"
            />
          </svg>
        </div>
        <select
          className="input sm:w-48"
          value={sort}
          onChange={(e) => navigate({ sort: e.target.value })}
          aria-label="Sort results"
        >
          {SORT_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="btn-primary shrink-0"
          onClick={() => setCreateOpen(true)}
        >
          Create app
        </button>
      </div>

      <FilterPanel
        facets={facets}
        selectedTags={selectedTags}
        onToggleTag={toggleTag}
        ranges={ranges}
        onRangeChange={onRangeChange}
        fuzzy={fuzzy}
        onFuzzyChange={onFuzzyChange}
        onClear={clearFilters}
      />

      <section>
        <div className="mb-3 flex items-center justify-between text-sm text-slate">
          <span>
            {loading ? "Searching…" : `${total} apps`}
            {query && !loading && ` for “${query}”`}
          </span>
        </div>

        {apps.length === 0 ? (
          <div
            key={
              query + JSON.stringify(ranges) + fuzzy + selectedTags.join(",")
            }
            className="card animate-fade-in grid place-items-center p-12 text-center text-slate-steel"
          >
            <p>No apps match your search.</p>
            <p className="mt-1 text-xs">
              Try removing a filter — or{" "}
              <button
                type="button"
                onClick={() => setCreateOpen(true)}
                className="text-cobalt hover:underline"
              >
                submit the app yourself
              </button>
              .
            </p>
          </div>
        ) : (
          <>
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {apps.map((app, index) => (
                <AppCard
                  key={app.id}
                  app={app}
                  rank={sort === "rank" ? index + 1 : undefined}
                />
              ))}
            </div>

            {hasMore && (
              <div
                ref={sentinelRef}
                className="mt-6 flex items-center justify-center py-4 text-sm text-slate-steel"
              >
                {loadingMore && "Loading more…"}
              </div>
            )}
          </>
        )}
      </section>

      <Modal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        title="Create app"
        maxWidthClassName="max-w-3xl"
      >
        <CreateAppForm
          onSuccess={() => {
            setCreateOpen(false);
            setRefreshKey((k) => k + 1);
          }}
        />
      </Modal>
    </div>
  );
}
