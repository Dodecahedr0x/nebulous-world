"use client";

import { useEffect, useRef, useState } from "react";
import { hierarchy, pack, type HierarchyCircularNode, type HierarchyNode } from "d3-hierarchy";
import { cn, formatToken, formatNumber } from "@/lib/utils";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { buildTagPackTree, type PackNode, type PackTagNode, type PackAppNode } from "@/lib/tagPack";
import type { TagPack, MapRangeFilters } from "@/lib/indexerClient";
import type { MapSelection } from "./RelatedApps";

// Representative fallback so the map is never empty if the API route is
// unreachable — same idea as TagMap/AppMap's FALLBACK_NODES.
const FALLBACK_PACK: TagPack = {
  tags: [
    { slug: "defi", name: "defi", appCount: 3, stake: 42000 },
    { slug: "nft", name: "nft", appCount: 2, stake: 18500 },
    { slug: "gaming", name: "gaming", appCount: 1, stake: 26000 },
    { slug: "wallet", name: "wallet", appCount: 1, stake: 22000 },
    { slug: "marketplace", name: "marketplace", appCount: 1, stake: 14300 },
  ],
  apps: [
    { slug: "jupiter", name: "Jupiter", stake: 38000, tagSlugs: ["defi"] },
    { slug: "kamino", name: "Kamino", stake: 29500, tagSlugs: ["defi", "wallet"] },
    { slug: "marinade", name: "Marinade", stake: 24100, tagSlugs: ["defi"] },
    { slug: "tensor", name: "Tensor", stake: 17600, tagSlugs: ["nft", "marketplace"] },
    { slug: "magic-eden", name: "Magic Eden", stake: 16200, tagSlugs: ["nft"] },
    { slug: "star-atlas", name: "Star Atlas", stake: 12400, tagSlugs: ["gaming"] },
  ],
};

// Both opaque (unlike the old alpha-blended fill) so a nested tag circle's
// color never compounds with whatever's behind it — d3.pack draws every
// circle in a fully-enclosed parent/child stack, so an alpha fill here
// would darken with each extra level of nesting regardless of this
// function's own depth/maxDepth interpolation, eventually going dark
// enough that LABEL_INK (a light-background color, see its own comment)
// stopped reading against it. Both endpoints are light enough to keep
// LABEL_INK's contrast comfortably above WCAG AA (4.5:1) at every depth —
// TAG_FILL_SHALLOW is the exact `indigo-soft` design token already used by
// `.chip-active` in globals.css, so a shallow tag circle matches that
// existing "selected filter" chip look.
const TAG_FILL_SHALLOW: [number, number, number] = [238, 240, 253];
const TAG_FILL_DEEP: [number, number, number] = [216, 219, 250];
// The brand's cobalt accent — same value as .btn-primary's bg-cobalt.
const APP_FILL = "#4338ca";
const SELECTED_RING = "#372fb0";
// Tag circles are always light (see TAG_FILL_SHALLOW/DEEP above), so a dark
// ink reads clearly on every one of them regardless of depth — this used to
// also be the app-leaf label color, but APP_FILL is a solid, saturated
// cobalt fill that near-black text has nowhere near enough contrast
// against (WCAG ratio ~2.5:1, well under the 4.5:1 AA minimum for text
// this size) — see APP_LABEL_INK for that case instead.
const LABEL_INK = "#0d0e12";
// App leaves sit on a solid cobalt (APP_FILL) circle — the same light
// "text-cream" color .btn-primary already pairs with that exact background
// color elsewhere in the design system, comfortably >7:1 contrast.
const APP_LABEL_INK = "#ffffff";
const MAX_SIBLINGS = 6;
// Floor for an app's normalized pack value (see the `appPackValue` comment
// at its call site) — a fraction of the biggest app's own value (which is
// always exactly 1 after normalization), so a 0-stake app's circle stays a
// fixed, legible proportion of the map's biggest circle no matter how big
// that biggest one is.
const APP_MIN_VALUE_FRACTION = 0.18;
const APP_LABEL_FONT_SIZE = 11;
const APP_LABEL_FONT_WEIGHT = 500;
const TAG_LABEL_FONT_SIZE = 12;
const TAG_LABEL_FONT_WEIGHT = 600;
// How far below a tag circle's topmost point its label sits — children
// fill the middle of a tag circle, so the label lives in that top rim
// instead of dead center like a leaf's does.
const TAG_LABEL_Y_OFFSET = 14;
// Horizontal breathing room a label needs on each side before it counts as
// "fits" — without this, a label could render right up against (or past)
// its own circle's stroke, which reads as cramped/cut-off even when it
// technically doesn't overlap a neighbor.
const LABEL_PADDING_PX = 6;
// Pan/zoom tuning — same shape as ForceMap's view transform, adapted to a
// declarative SVG `<g>` (a CSS `transform` + `transition` pair) instead of
// an imperative canvas repaint loop, since GroupMap has no per-frame work.
const MIN_ZOOM = 0.6;
const MAX_ZOOM = 8;
const ZOOM_BUTTON_FACTOR = 1.5;
// Fraction of the viewport a zoomed-to node's diameter should fill — leaves
// a little breathing room instead of touching the edges exactly.
const FOCUS_FIT_FACTOR = 0.86;
// A pointer that moved less than this while panning still counts as a
// click (select/zoom), not a drag — same idea as ForceMap's
// CLICK_DRAG_THRESHOLD_PX.
const CLICK_DRAG_THRESHOLD_PX = 4;
// How long a wheel/drag gesture suppresses the CSS transition for, after
// the last input event — long enough that a burst of wheel ticks or a
// pointermove stream never fights the animated (click-to-zoom) case, short
// enough that letting go feels immediate rather than sluggish.
const GESTURE_SETTLE_MS = 200;

function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v));
}

function tagFill(depth: number, maxDepth: number): string {
  const t = maxDepth > 0 ? depth / maxDepth : 0;
  const [r1, g1, b1] = TAG_FILL_SHALLOW;
  const [r2, g2, b2] = TAG_FILL_DEEP;
  const r = Math.round(r1 + (r2 - r1) * t);
  const g = Math.round(g1 + (g2 - g1) * t);
  const b = Math.round(b1 + (b2 - b1) * t);
  return `rgb(${r}, ${g}, ${b})`;
}

// Lazily created, cached — a single offscreen canvas 2D context is enough
// to measure every label on every render; there's no reason to allocate
// one per node or per render.
let measureCtx: CanvasRenderingContext2D | null | undefined;
function getMeasureCtx(): CanvasRenderingContext2D | null {
  if (measureCtx === undefined) {
    measureCtx = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  }
  return measureCtx ?? null;
}

// Actual rendered pixel width of a label, not a character-count guess —
// this is what lets the fit checks below be exact instead of a flat radius
// cutoff that let long names overflow small-but-above-threshold circles
// while hiding short names on circles that had plenty of room to spare.
// `document` isn't available during SSR; the fallback is a deliberately
// generous per-character estimate so a label is never claimed to fit when
// it might not (better to hide once on hydration than flash overflowing
// text for one server-rendered frame).
function measureLabelWidth(text: string, fontSize: number, fontWeight: number): number {
  const ctx = getMeasureCtx();
  if (!ctx) return text.length * fontSize * 0.65;
  ctx.font = `${fontWeight} ${fontSize}px ui-sans-serif, system-ui, sans-serif`;
  return ctx.measureText(text).width;
}

// Half the horizontal chord available at TAG_LABEL_Y_OFFSET below a tag
// circle's topmost point — narrower than the full diameter, since the
// label doesn't sit at the circle's widest point. `r` here (like in the fit
// checks below) is an on-*screen* radius — see their own comment for why.
function tagLabelHalfWidth(r: number): number {
  const dy = r - TAG_LABEL_Y_OFFSET;
  return Math.sqrt(Math.max(0, r * r - dy * dy));
}

// `r` is the circle's EFFECTIVE on-screen radius (n.r * the current view
// zoom), not its raw layout radius. Labels are rendered at a constant
// on-screen size regardless of zoom (see the `/ view.k` counter-scaling at
// the call site — same technique ForceMap/this file already use to keep
// stroke widths a constant screen thickness), so whether a label fits is
// also a screen-space question: a circle too small for its label at the
// default zoom can still grow past that threshold once the user zooms in
// on it, which is the whole point of click-to-zoom existing. Checking the
// unzoomed layout radius instead would mean a label some apps could never
// be read even by zooming all the way in on them.
function appLabelFits(r: number, name: string): boolean {
  const available = 2 * r - LABEL_PADDING_PX * 2;
  return available > 0 && available >= measureLabelWidth(name, APP_LABEL_FONT_SIZE, APP_LABEL_FONT_WEIGHT);
}

function tagLabelFits(r: number, name: string): boolean {
  if (r <= TAG_LABEL_Y_OFFSET) return false;
  const available = tagLabelHalfWidth(r) * 2 - LABEL_PADDING_PX * 2;
  return available > 0 && available >= measureLabelWidth(`#${name}`, TAG_LABEL_FONT_SIZE, TAG_LABEL_FONT_WEIGHT);
}

type PackedNode = HierarchyCircularNode<PackNode>;
interface View {
  x: number;
  y: number;
  k: number;
}
const IDENTITY_VIEW: View = { x: 0, y: 0, k: 1 };

// A tag slug can legitimately appear at more than one depth in the tree —
// e.g. "wallet" both as its own top-level bucket (apps tagged only wallet)
// and nested under "defi" (apps tagged defi+wallet) — so `data.id` alone
// isn't a unique React key or a safe hover/selection identity. The full
// root-to-node path of ids is.
function nodeKey(n: PackedNode): string {
  return n
    .ancestors()
    .reverse()
    .map((a) => a.data.id)
    .join(">");
}

// The node one level up from `n` that's still part of the rendered tree
// (depth > 0) — i.e. where "zoom out one level" from `n` should land.
// Depth-1 nodes' parent is the synthetic root, which isn't itself
// rendered/zoomable, so that case (and the root itself) resolve to `null`,
// meaning "full view, no focus."
function parentFocusTarget(n: PackedNode): PackedNode | null {
  return n.parent && n.parent.depth > 0 ? n.parent : null;
}

function selectionFor(node: PackedNode): MapSelection | null {
  if (node.data.type === "app") {
    const app = node.data;
    const siblings = (node.parent?.children ?? [])
      .filter((c) => c.data.type === "app" && c.data.id !== app.id)
      .map((c) => (c.data as PackAppNode).id)
      .slice(0, MAX_SIBLINGS);
    return { kind: "app", label: app.name, slugs: [app.id, ...siblings], selectedSlug: app.id };
  }
  // A tag slug can appear at more than one node in the tree (e.g. "wallet"
  // both on its own and nested under "defi" — see nodeKey's comment), so
  // /api/apps/related's tagSlugs= (an OR match against every app carrying
  // that slug ANYWHERE) would over-select here, pulling in apps from a
  // sibling node that happens to share the tag but not the rest of this
  // node's ancestor path. GroupMap already knows the exact app set for this
  // circle locally — its leaves — so it resolves apps by exact slug instead
  // of asking the server to re-derive an ambiguous one. This also covers
  // the synthetic "untagged" bucket for free, which isn't a real Tag row
  // tagSlugs= could look up in the first place.
  const leafIds = node.leaves().map((l) => (l.data as PackAppNode).id);
  return leafIds.length > 0 ? { kind: "tag", label: node.data.name, slugs: leafIds } : null;
}

/**
 * Interactive D3 circle-packing view of the tag hierarchy synthesized in
 * tagPack.ts: outer circles are each app's most globally-common tag, inner
 * circles nest one tag deeper, and full (leaf) circles are individual apps.
 * Inspired by https://observablehq.com/@d3/zoomable-circle-packing. Unlike
 * AppMap/TagMap, the circles themselves have no physics step (their
 * position is deterministic from d3.pack()), so the "dynamic" part is
 * entirely a view transform layered on top: drag anywhere to pan, and
 * scroll/pinch or the +/−/reset buttons to zoom — the same interaction
 * language as ForceMap's canvas maps, just driven by a CSS
 * `transform`/`transition` on a wrapping `<g>` instead of a per-frame canvas
 * repaint, since there's no simulation to redraw every tick.
 *
 * Clicking a circle means one of two different things depending on what it
 * is, since a tag and an app aren't interchangeable here the way they are
 * in TagMap/AppMap: clicking an APP leaf zooms in and selects it (same as
 * before — a one-off preview, shown in RelatedApps below). Clicking a TAG
 * circle instead toggles that tag into/out of `selectedTags`, the same
 * standing filter ExploreMaps' chip picker drives — not a one-off zoom,
 * since once a tag is filtered the tree is rebuilt from only the apps that
 * carry it, and the existing "reset view when the data changes" effect
 * already snaps to a fresh full view of that narrower tree, which serves
 * "look closer at this tag" better than a manual zoom would (no unrelated
 * apps left cluttering the view to zoom past). Deliberately not both at
 * once — toggling the filter already changes the tree the old zoom target
 * would have pointed into, and showing a RelatedApps preview for a tag
 * whose apps are already the entire (filtered) map would be redundant.
 */
export function GroupMap({
  onSelect,
  selectedTags = [],
  onToggleTag,
  ranges,
}: {
  onSelect?: (selection: MapSelection | null) => void;
  selectedTags?: string[];
  onToggleTag?: (slug: string) => void;
  /** Advanced-search range filters (min/max app stake, tag count, pageviews) — applied server-side, see /api/tags/pack. */
  ranges?: MapRangeFilters;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  const [source, setSource] = useState<"live" | "sample">("sample");
  const [loading, setLoading] = useState(true);
  const [isEmpty, setIsEmpty] = useState(false);
  const [pkg, setPkg] = useState<TagPack>(FALLBACK_PACK);
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<View>(IDENTITY_VIEW);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  // True while a wheel/drag gesture is live (or has just ended) — the `<g>`
  // skips its CSS transition then, so 1:1 pointer/wheel tracking never lags
  // behind the input, while a click-to-zoom (state-driven, no live pointer
  // to track) still animates smoothly.
  const [instant, setInstant] = useState(false);
  const [isPanning, setIsPanning] = useState(false);
  const gestureTimeoutRef = useRef<number | null>(null);
  const panStartRef = useRef({ pointerX: 0, pointerY: 0, viewX: 0, viewY: 0 });
  const draggedRef = useRef(false);

  const rangesKey = JSON.stringify(ranges ?? {});
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setIsEmpty(false);
    const sp = new URLSearchParams();
    for (const [key, value] of Object.entries(ranges ?? {})) {
      if (value) sp.set(key, value);
    }
    const qs = sp.toString();
    fetch(`/api/tags/pack${qs ? `?${qs}` : ""}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((body: { data?: TagPack }) => {
        if (cancelled) return;
        const data = body.data;
        if (!data?.apps) {
          setLoading(false);
          setSource("sample");
          setPkg(FALLBACK_PACK);
          return;
        }
        if (data.apps.length === 0) {
          setSource("live");
          setLoading(false);
          setIsEmpty(true);
          setPkg(data);
          return;
        }
        setSource("live");
        setLoading(false);
        setPkg(data);
      })
      .catch(() => {
        if (cancelled) return;
        setLoading(false);
        setSource("sample");
        setPkg(FALLBACK_PACK);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rangesKey]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const rect = el.getBoundingClientRect();
      setSize({ width: rect.width, height: rect.height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // A resize, a fresh dataset, or a tag-filter change all change every
  // node's x/y/r (pack() re-lays out from scratch each time), so a pan/zoom
  // computed against the old layout no longer points at anything
  // meaningful — reset to the full view rather than leave the camera aimed
  // at empty space. A stale selection is cleared for the same reason: it
  // might reference an app the new filter just excluded from the tree
  // entirely.
  const selectedTagsKey = selectedTags.join(",");
  useEffect(() => {
    setView(IDENTITY_VIEW);
    setFocusKey(null);
    setSelectedId(null);
    onSelect?.(null);
    // onSelect intentionally excluded — it's a fresh closure every render
    // (ExploreMaps doesn't memoize its handlers), and including it would
    // re-run this reset (clearing the very selection it just set) on every
    // parent re-render, not just when the filter/dataset actually changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [size.width, size.height, pkg, selectedTagsKey]);

  // Native, non-passive listener so preventDefault reliably stops page
  // scroll while the cursor is over the map — React's onWheel is passive by
  // default and can't do this (same reasoning as ForceMap's onWheel).
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg) return;
    function onWheel(e: WheelEvent) {
      e.preventDefault();
      const rect = svg!.getBoundingClientRect();
      const px = e.clientX - rect.left;
      const py = e.clientY - rect.top;
      const factor = Math.exp(-e.deltaY * 0.0015);
      beginInstantGesture();
      setFocusKey(null);
      setView((v) => {
        const nextK = clamp(v.k * factor, MIN_ZOOM, MAX_ZOOM);
        const applied = nextK / v.k;
        return { k: nextK, x: px - (px - v.x) * applied, y: py - (py - v.y) * applied };
      });
    }
    svg.addEventListener("wheel", onWheel, { passive: false });
    return () => svg.removeEventListener("wheel", onWheel);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function beginInstantGesture() {
    setInstant(true);
    if (gestureTimeoutRef.current) window.clearTimeout(gestureTimeoutRef.current);
    gestureTimeoutRef.current = window.setTimeout(() => setInstant(false), GESTURE_SETTLE_MS);
  }

  // AND semantics — same as AppMap's own tag filter (see its fetchUrl's
  // ?tags= building an intersection server-side): a filtered map should
  // only ever get more specific as more tags are added, not less.
  // `tags` in the tree still comes from the full, unfiltered `pkg` — an
  // app's own tags are still ranked by their TRUE global popularity across
  // every app, not just the filtered subset, so filtering to "wallet"
  // narrows which apps appear without reshuffling how the ones that remain
  // nest relative to each other.
  const filteredApps =
    selectedTags.length > 0
      ? pkg.apps.filter((a) => selectedTags.every((t) => a.tagSlugs.includes(t)))
      : pkg.apps;
  const isFilteredEmpty = selectedTags.length > 0 && filteredApps.length === 0 && !isEmpty;
  const tree = buildTagPackTree({ tags: pkg.tags, apps: filteredApps });
  const root: PackTagNode = { type: "tag", id: "__root__", name: "", children: tree.children };
  const width = Math.max(1, size.width);
  const height = Math.max(1, size.height);
  // d3.pack sizes a leaf's AREA proportional to its `.sum()` value, i.e.
  // radius ∝ sqrt(value) — feeding it raw stake directly (as `Math.max(1,
  // d.stake)` used to) means radius ends up ∝ sqrt(stake), and real stake
  // distributions are heavily power-law: an app with 1 (the old floor) next
  // to one with 40,000 gets a radius ~1/200th the size, an unreadable
  // pinprick rather than a small circle. Normalizing against the biggest
  // app currently on the map (an additional sqrt on top of pack's own,
  // i.e. final radius ∝ stake^0.25 relative to the max) compresses that
  // range dramatically while still ordering apps correctly by stake, and
  // flooring the normalized value (not the raw stake) guarantees a
  // meaningful minimum circle size — smallestRadius/biggestRadius works out
  // to sqrt(APP_MIN_VALUE_FRACTION) ≈ 42%, comfortably legible — regardless
  // of whether "small" means 0 stake or just much less than this map's
  // biggest app.
  const maxAppStake = Math.max(1, ...filteredApps.map((a) => a.stake));
  const appPackValue = (stake: number) =>
    Math.max(APP_MIN_VALUE_FRACTION, Math.sqrt(Math.max(0, stake) / maxAppStake));
  const h = hierarchy<PackNode>(root, (d) => (d.type === "tag" ? d.children : undefined))
    .sum((d) => (d.type === "app" ? appPackValue(d.stake) : 0))
    .sort((a, b) => (b.value ?? 0) - (a.value ?? 0));
  // `padding` isn't just a gap between SIBLING circles — d3 reserves it
  // around every child before enclosing them in their parent, so it's also
  // what separates a tag's own label (which sits at its circle's own top
  // rim) from its first child's label (which sits at THAT circle's own top
  // rim, only `padding` below it). At the old value of 3 the two labels
  // ended up close enough to render on top of each other — illegible
  // without zooming in until the (screen-constant-sized) text finally had
  // room. 8 was chosen empirically: enough clearance to keep parent/child
  // and adjacent-sibling labels apart even on realistically dense maps
  // (tested at 12 tags/45 apps and a 18-tag/90-app stress case), without
  // shrinking circles enough to push any label below tagLabelFits'/
  // appLabelFits' fit threshold on a normal-sized map that read fine before.
  const packed = pack<PackNode>().size([width, height]).padding(8)(h as HierarchyNode<PackNode>) as PackedNode;
  const nodes = packed.descendants().filter((d) => d.depth > 0);
  const maxDepth = nodes.reduce((m, d) => Math.max(m, d.depth), 1);
  const hovered = hoveredId ? nodes.find((n) => nodeKey(n) === hoveredId) ?? null : null;
  const leafNodes = nodes.filter((n): n is PackedNode & { data: PackAppNode } => n.data.type === "app");

  // Sets the view so `node` fills most of the viewport, or resets to the
  // full pack when `node` is null — the click-to-zoom half of the
  // interaction; drag/wheel below drive the same `view` state directly.
  function zoomToNode(node: PackedNode | null) {
    if (!node) {
      setView(IDENTITY_VIEW);
      setFocusKey(null);
      return;
    }
    const k = clamp((Math.min(width, height) / (node.r * 2)) * FOCUS_FIT_FACTOR, MIN_ZOOM, MAX_ZOOM);
    setView({ x: width / 2 - node.x * k, y: height / 2 - node.y * k, k });
    setFocusKey(nodeKey(node));
  }

  function zoomByFactor(factor: number) {
    beginInstantGesture();
    setFocusKey(null);
    setView((v) => {
      const nextK = clamp(v.k * factor, MIN_ZOOM, MAX_ZOOM);
      const applied = nextK / v.k;
      return {
        k: nextK,
        x: width / 2 - (width / 2 - v.x) * applied,
        y: height / 2 - (height / 2 - v.y) * applied,
      };
    });
  }

  function select(node: PackedNode) {
    const key = nodeKey(node);
    setSelectedId(key);
    onSelect?.(selectionFor(node));
  }

  // Click-to-zoom: clicking an app leaf zooms in and selects it; clicking
  // the leaf that's already filling the view zooms back out one level
  // instead (and drops the selection, mirroring a background click).
  function handleAppClick(n: PackedNode) {
    const key = nodeKey(n);
    if (focusKey === key) {
      zoomToNode(parentFocusTarget(n));
      setSelectedId(null);
      onSelect?.(null);
      return;
    }
    zoomToNode(n);
    select(n);
  }

  // A tag circle's click means "filter to this tag" rather than "zoom to
  // this circle" — see the component doc comment for why the two node
  // types don't share a click behavior here. No-ops if the caller didn't
  // wire up filtering (onToggleTag is optional).
  function handleCircleClick(n: PackedNode) {
    if (n.data.type === "app") {
      handleAppClick(n);
    } else {
      onToggleTag?.(n.data.id);
    }
  }

  // Screen (client) coordinates -> the same node-space (n.x/n.y/n.r) our
  // pack layout is computed in, undoing both the SVG's own CSS→viewBox
  // scaling and our pan/zoom `view` transform.
  function screenToNodeSpace(clientX: number, clientY: number): { x: number; y: number } | null {
    const svg = svgRef.current;
    if (!svg) return null;
    const rect = svg.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return null;
    const svgX = ((clientX - rect.left) / rect.width) * width;
    const svgY = ((clientY - rect.top) / rect.height) * height;
    return { x: (svgX - view.x) / view.k, y: (svgY - view.y) / view.k };
  }

  // The deepest (most specific/innermost) node whose circle contains
  // `point` — circles nest, so a click point sitting inside a leaf app
  // circle is necessarily also inside every one of its ancestor tag
  // circles; the innermost one is the one a viewer would say they clicked.
  function nodeAtPoint(point: { x: number; y: number }): PackedNode | null {
    let best: PackedNode | null = null;
    for (const n of nodes) {
      const dx = point.x - n.x;
      const dy = point.y - n.y;
      if (dx * dx + dy * dy <= n.r * n.r && (!best || n.depth > best.depth)) best = n;
    }
    return best;
  }

  // Resolving clicks by DOM element (a native `onClick` per node circle)
  // doesn't work here: `onPointerDownPan` below calls setPointerCapture on
  // the SVG so panning keeps tracking even if the pointer leaves it
  // mid-drag, but that capture ALSO retargets the resulting synthetic
  // `click` event's target to the capturing element (the SVG) instead of
  // whatever circle was actually under the pointer — a well-known Pointer
  // Events quirk. So click resolution is done manually here, in the
  // pointerup handler, the same way ForceMap's canvas (which has no DOM
  // nodes to click at all) hit-tests its own nodes.
  function onPointerDownPan(e: React.PointerEvent<SVGSVGElement>) {
    if (e.button !== 0 && e.pointerType === "mouse") return;
    draggedRef.current = false;
    panStartRef.current = { pointerX: e.clientX, pointerY: e.clientY, viewX: view.x, viewY: view.y };
    e.currentTarget.setPointerCapture(e.pointerId);
    setIsPanning(true);
  }
  function onPointerMovePan(e: React.PointerEvent<SVGSVGElement>) {
    if (!isPanning) return;
    const dx = e.clientX - panStartRef.current.pointerX;
    const dy = e.clientY - panStartRef.current.pointerY;
    if (!draggedRef.current && Math.hypot(dx, dy) > CLICK_DRAG_THRESHOLD_PX) {
      draggedRef.current = true;
      setInstant(true);
      setFocusKey(null);
    }
    if (!draggedRef.current) return;
    setView((v) => ({ ...v, x: panStartRef.current.viewX + dx, y: panStartRef.current.viewY + dy }));
  }
  function onPointerUpPan(e: React.PointerEvent<SVGSVGElement>) {
    setIsPanning(false);
    if (draggedRef.current) {
      beginInstantGesture();
    } else {
      const point = screenToNodeSpace(e.clientX, e.clientY);
      const hit = point && nodeAtPoint(point);
      if (hit) {
        handleCircleClick(hit);
      } else if (selectedId || focusKey) {
        zoomToNode(null);
        setSelectedId(null);
        onSelect?.(null);
      }
    }
    // Cleared on the next tick rather than immediately so any other handler
    // still reading this gesture's outcome later in the same tick sees it.
    window.setTimeout(() => {
      draggedRef.current = false;
    }, 0);
  }

  return (
    <div>
      <div className="relative">
        <div
          ref={containerRef}
          className="explore-map-surface relative h-[24rem] w-full overflow-hidden rounded-card border border-hairline sm:h-[30rem]"
        >
          <svg
            ref={svgRef}
            width="100%"
            height="100%"
            viewBox={`0 0 ${width} ${height}`}
            role="img"
            aria-label="Circle-packing map of nebulous.world tags and apps. Outer circles are the most common tags; inner circles nest one tag deeper; small filled circles are individual apps. Drag to pan, scroll or use the zoom buttons to zoom. Click an app to zoom into it and select it; click a tag to filter the map down to apps carrying it."
            className={cn("touch-none select-none", isPanning ? "cursor-grabbing" : "cursor-grab")}
            onPointerDown={onPointerDownPan}
            onPointerMove={onPointerMovePan}
            onPointerUp={onPointerUpPan}
          >
            <g
              style={{
                transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})`,
                transformOrigin: "0px 0px",
                transition: instant ? "none" : "transform 480ms cubic-bezier(0.22, 1, 0.36, 1)",
              }}
            >
              {nodes.map((n) => {
                const key = nodeKey(n);
                const isApp = n.data.type === "app";
                const isSelected = selectedId === key;
                const isHovered = hoveredId === key;
                // A tag circle's "selected" state is standing (part of the
                // active filter) rather than transient like an app leaf's —
                // it stays highlighted until toggled off again, not just
                // while it's the last-clicked thing.
                const isFilterTag = !isApp && selectedTags.includes(n.data.id);
                // Effective on-screen radius at the current zoom — see
                // appLabelFits' comment for why the fit check (and the
                // counter-scaled fontSize/y below) use this instead of n.r.
                const screenR = n.r * view.k;
                const showLabel = isApp ? appLabelFits(screenR, n.data.name) : tagLabelFits(screenR, n.data.name);
                return (
                  <g
                    key={key}
                    transform={`translate(${n.x}, ${n.y})`}
                    onPointerEnter={() => setHoveredId(key)}
                    onPointerLeave={() => setHoveredId((id) => (id === key ? null : id))}
                    className="cursor-pointer"
                  >
                    <circle
                      r={n.r}
                      fill={isApp ? APP_FILL : tagFill(n.depth, maxDepth)}
                      fillOpacity={isApp ? (isHovered || isSelected ? 0.95 : 0.75) : undefined}
                      stroke={isSelected || isFilterTag ? SELECTED_RING : isHovered ? "#0d0e12" : "rgba(13,14,18,0.15)"}
                      strokeWidth={(isSelected || isFilterTag ? 2.5 : isHovered ? 2 : 1) / view.k}
                    />
                    {showLabel && (
                      <text
                        textAnchor="middle"
                        y={(isApp ? 4 : -screenR + TAG_LABEL_Y_OFFSET) / view.k}
                        fontSize={(isApp ? APP_LABEL_FONT_SIZE : TAG_LABEL_FONT_SIZE) / view.k}
                        fontWeight={isApp ? APP_LABEL_FONT_WEIGHT : TAG_LABEL_FONT_WEIGHT}
                        fill={isApp ? APP_LABEL_INK : LABEL_INK}
                        className="pointer-events-none select-none"
                      >
                        {isApp ? n.data.name : `#${n.data.name}`}
                      </text>
                    )}
                  </g>
                );
              })}
            </g>
          </svg>
        </div>
        {isEmpty && (
          <div className="pointer-events-none absolute inset-0 grid place-items-center px-6 text-center">
            <p className="text-sm text-slate-steel">No approved apps to group yet.</p>
          </div>
        )}
        {isFilteredEmpty && (
          <div className="pointer-events-none absolute inset-0 grid place-items-center px-6 text-center">
            <p className="text-sm text-slate-steel">
              No apps carry every selected tag: {selectedTags.map((t) => `#${t}`).join(", ")}.
            </p>
          </div>
        )}
        {loading && !isEmpty && (
          <div className="pointer-events-none absolute inset-0 grid place-items-center">
            <p className="text-sm text-slate-steel">Loading…</p>
          </div>
        )}
        {hovered && (
          <div className="pointer-events-none absolute left-3 top-3 max-w-[14rem] rounded-card border border-hairline bg-cream/90 px-3 py-2 backdrop-blur-sm">
            <div className="text-sm font-semibold text-ink">
              {hovered.data.type === "app" ? hovered.data.name : `#${hovered.data.name}`}
            </div>
            {hovered.data.type === "app" ? (
              <div className="text-caption text-slate">{formatToken(hovered.data.stake, TOKEN_SYMBOL)} staked</div>
            ) : (
              <div className="text-caption text-slate">{formatNumber(hovered.leaves().length)} apps</div>
            )}
          </div>
        )}
        <div className="absolute bottom-3 right-3 flex flex-col overflow-hidden rounded-card border border-hairline bg-cream/80 backdrop-blur-sm">
          <button
            type="button"
            onClick={() => zoomByFactor(ZOOM_BUTTON_FACTOR)}
            aria-label="Zoom in"
            className="grid h-10 w-10 place-items-center text-slate transition-colors duration-150 hover:bg-mist"
          >
            +
          </button>
          <div className="h-px bg-hairline" />
          <button
            type="button"
            onClick={() => zoomByFactor(1 / ZOOM_BUTTON_FACTOR)}
            aria-label="Zoom out"
            className="grid h-10 w-10 place-items-center text-slate transition-colors duration-150 hover:bg-mist"
          >
            −
          </button>
          <div className="h-px bg-hairline" />
          <button
            type="button"
            onClick={() => {
              zoomToNode(null);
              setSelectedId(null);
              onSelect?.(null);
            }}
            aria-label="Reset zoom and pan"
            className="grid h-10 w-10 place-items-center text-sm text-slate transition-colors duration-150 hover:bg-mist"
          >
            ⟲
          </button>
        </div>
      </div>
      <ul className="sr-only">
        {leafNodes.map((n) => (
          <li key={n.data.id}>
            {onSelect ? (
              <button type="button" onClick={() => handleAppClick(n)}>
                {n.data.name}: {formatToken(n.data.stake, TOKEN_SYMBOL)} staked
              </button>
            ) : (
              `${n.data.name}: ${formatToken(n.data.stake, TOKEN_SYMBOL)} staked`
            )}
          </li>
        ))}
      </ul>
      <div className="mt-2 text-caption text-slate-steel">
        {source === "live" ? "Live tags & apps." : "Sample tags & apps."} Drag to pan, scroll to zoom, click an app
        to zoom in and select it, click a tag to filter the map to it.
      </div>
    </div>
  );
}
