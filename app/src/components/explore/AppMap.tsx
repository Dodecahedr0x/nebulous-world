"use client";

import { formatToken, formatNumber } from "@/lib/utils";
import { TOKEN_SYMBOL } from "@/lib/constants";
import type { AppGraph, MapRangeFilters } from "@/lib/indexerClient";
type AppGraphNode = AppGraph["nodes"][number];
type AppGraphEdge = AppGraph["edges"][number];
import { ForceMap, type MapLink, type MapNode } from "./ForceMap";

// Representative fallback so the map is never empty if the API route is
// unreachable — same shape as buildAppGraph() in src/lib/appGraph.ts.
const FALLBACK_NODES: MapNode[] = [
  { id: "jupiter", label: "Jupiter", metrics: { stake: 38000, views: 92000, votes: 15400 } },
  { id: "kamino", label: "Kamino", metrics: { stake: 29500, views: 61000, votes: 11200 } },
  { id: "tensor", label: "Tensor", metrics: { stake: 17600, views: 54000, votes: 8900 } },
  { id: "magic-eden", label: "Magic Eden", metrics: { stake: 16200, views: 71000, votes: 9700 } },
  { id: "marinade", label: "Marinade", metrics: { stake: 24100, views: 39000, votes: 7300 } },
  { id: "realms", label: "Realms", metrics: { stake: 9800, views: 21000, votes: 3600 } },
  { id: "star-atlas", label: "Star Atlas", metrics: { stake: 12400, views: 33000, votes: 4900 } },
  { id: "phantom", label: "Phantom", metrics: { stake: 33200, views: 128000, votes: 19800 } },
];
const FALLBACK_LINKS: MapLink[] = [
  { source: "jupiter", target: "kamino", metrics: { shared: 0.4, weighted: 0.55 } },
  { source: "jupiter", target: "marinade", metrics: { shared: 0.35, weighted: 0.48 } },
  { source: "kamino", target: "marinade", metrics: { shared: 0.3, weighted: 0.4 } },
  { source: "tensor", target: "magic-eden", metrics: { shared: 0.5, weighted: 0.6 } },
  { source: "tensor", target: "star-atlas", metrics: { shared: 0.22, weighted: 0.28 } },
  { source: "magic-eden", target: "star-atlas", metrics: { shared: 0.25, weighted: 0.3 } },
  { source: "phantom", target: "jupiter", metrics: { shared: 0.28, weighted: 0.35 } },
  { source: "phantom", target: "magic-eden", metrics: { shared: 0.2, weighted: 0.22 } },
  { source: "realms", target: "marinade", metrics: { shared: 0.3, weighted: 0.32 } },
];

/**
 * Which apps overlap most in what they're tagged as — a way to find
 * something similar to an app you already like. Click a node to select it
 * and see it (and its closest neighbors) listed below the map.
 * `selectedTags`, if given, restricts the map to apps carrying every one of
 * those tags (see buildAppGraph's doc comment for why this is AND, not OR).
 */
export function AppMap({
  onSelect,
  selectedTags = [],
  ranges,
}: {
  onSelect?: (node: MapNode | null, neighborIds: string[]) => void;
  selectedTags?: string[];
  /** Advanced-search range filters (min/max app stake, tag count, pageviews) — applied server-side, see /api/apps/graph. */
  ranges?: MapRangeFilters;
}) {
  const sp = new URLSearchParams();
  if (selectedTags.length > 0) sp.set("tags", selectedTags.join(","));
  for (const [key, value] of Object.entries(ranges ?? {})) {
    if (value) sp.set(key, value);
  }
  const qs = sp.toString();
  const fetchUrl = `/api/apps/graph${qs ? `?${qs}` : ""}`;

  return (
    <ForceMap<AppGraphNode, AppGraphEdge>
      fetchUrl={fetchUrl}
      mapNode={(n) => ({ id: n.id, label: n.name, metrics: { stake: n.stake, views: n.views, votes: n.votes } })}
      mapLink={(l) => ({ source: l.source, target: l.target, metrics: { shared: l.shared, weighted: l.weighted } })}
      fallbackNodes={selectedTags.length > 0 ? [] : FALLBACK_NODES}
      fallbackLinks={selectedTags.length > 0 ? [] : FALLBACK_LINKS}
      sourceLabel="apps"
      ariaLabel="Map of nebulous.world apps, grouped by how similar their tags are. Circle size depends on the selected option — by default, total stake. Click an app to select it and see related apps below; drag a node to reposition, drag the background to pan, scroll to zoom."
      onSelect={onSelect}
      emptyMessage={
        selectedTags.length > 0
          ? `No apps carry every selected tag: ${selectedTags.map((t) => `#${t}`).join(", ")}.`
          : undefined
      }
      sizeMetrics={[
        { key: "stake", label: "Stake", format: (v) => `${formatToken(v, TOKEN_SYMBOL)} staked` },
        { key: "views", label: "Page views", format: (v) => `${formatNumber(v)} views` },
        { key: "votes", label: "Votes", format: (v) => `${formatToken(v, TOKEN_SYMBOL)} votes` },
      ]}
      linkMetrics={[
        { key: "shared", label: "Shared tags" },
        { key: "weighted", label: "Shared tags, weighted by stake" },
      ]}
    />
  );
}
