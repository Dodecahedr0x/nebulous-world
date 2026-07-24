"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";
import { cn } from "@/lib/utils";

export interface MapNode extends SimulationNodeDatum {
  id: string;
  label: string;
  metrics: Record<string, number>;
}
export interface MapLink extends SimulationLinkDatum<MapNode> {
  metrics: Record<string, number>;
}
export interface MapMetric {
  key: string;
  label: string;
  // Only rendered for size metrics (the tooltip and sr-only list show a
  // node's size-metric values, never its link-metric ones), so link
  // metrics can omit this.
  format?: (value: number) => string;
}

interface ForceMapProps<RawNode, RawLink> {
  fetchUrl: string;
  // The API returns flat, domain-shaped records (e.g. { id, name, stake }) —
  // these adapt a fetched record into the { id, label, metrics } shape
  // ForceMap actually renders, keeping the domain types (TagGraphNode,
  // AppGraphNode, ...) as the source of truth instead of duplicating them.
  mapNode: (raw: RawNode) => MapNode;
  mapLink: (raw: RawLink) => MapLink;
  fallbackNodes: MapNode[];
  fallbackLinks: MapLink[];
  sizeMetrics: MapMetric[];
  linkMetrics: MapMetric[];
  ariaLabel: string;
  sourceLabel: string;
  // Fired when a node is clicked (selecting it) or when the background is
  // clicked/an already-selected node is clicked again (deselecting, called
  // with null). `neighborIds` is every node directly connected to `node` —
  // "connected peers" — so callers can build a related-items list without
  // re-deriving adjacency themselves.
  onSelect?: (node: MapNode | null, neighborIds: string[]) => void;
  // Shown instead of the canvas when a LIVE fetch succeeds but returns zero
  // nodes (e.g. a tag-combination filter that matches nothing) — distinct
  // from a fetch failure, which still falls back to sample data, since a
  // genuinely empty result shouldn't be masked by unrelated sample nodes.
  emptyMessage?: string;
  // Programmatically selects the node with this id — e.g. a caller-side
  // search/autocomplete picking a node without the user clicking it
  // directly, or a URL query param seeding the initial selection (see
  // TagChip/ExploreMaps) before this map has even fetched its data yet — see
  // `pendingSelectRequestRef` for how that still applies once the
  // simulation actually starts, instead of silently no-op-ing. A fresh
  // object each time (not just a changed `id`) is what triggers a NEW
  // selection: selection state lives inside this component's own imperative
  // canvas closure, not in React state a parent could otherwise diff
  // against, so re-selecting the SAME id twice in a row (e.g. picking it
  // again after clicking a different node on the canvas directly in
  // between) needs a new object reference to be noticed at all — see the
  // effect below.
  selectRequest?: { id: string } | null;
}

// DESIGN.md tokens (see globals.css/tailwind.config.ts): indigo for
// nodes/edges, a darker indigo for the selection ring, border/muted-ink for
// dimmed states — restyled for the light canvas, same values used
// elsewhere in the app rather than one-off hex.
const NODE_FILL = "#4338ca";
const NODE_FILL_DIM = "#d1d3da";
const EDGE_STROKE = "#a5a8b8";
const LABEL_INK = "#0d0e12";
const LABEL_DIM = "#565a66";
const SELECTED_RING = "#372fb0";
// Local light-glass chip styling for this component's own metric pickers —
// deliberately not the shared `.chip`/`.chip-active` classes, which assume
// an opaque card surface behind them, unlike this canvas overlay's
// translucent panel over the map background.
const DARK_CHIP =
  "inline-flex items-center gap-1 rounded-pill border border-hairline bg-cream/80 px-2.5 py-1 text-xs text-slate transition-colors duration-150 hover:bg-mist";
const DARK_CHIP_ACTIVE = "border-cobalt/50 bg-indigo-soft text-cobalt";
// A pointer that moved less than this while a node was grabbed (or the
// background was pressed) still counts as a click/tap, not a drag/pan —
// real pointers rarely stay perfectly still.
const CLICK_DRAG_THRESHOLD_PX = 4;
// Node radius range, in CSS px — wide enough that the smallest and largest
// nodes in a typical power-law distribution (a few heavily-staked/tagged
// items, a long tail of small ones) read as clearly different at a glance,
// not just "a bit bigger." Radius uses sqrt(fraction) so on-screen AREA
// (the visual quantity a viewer actually compares) scales closer to
// linearly with the underlying metric than raw radius would.
const MIN_RADIUS = 5;
const RADIUS_RANGE = 33;
const MIN_ZOOM = 0.4;
const MAX_ZOOM = 4;
const ZOOM_BUTTON_FACTOR = 1.4;
// Edges render as a tapered spindle — thick near each endpoint node, thin at
// the midpoint — rather than a uniform-width line, so a glance can follow a
// link back to the node it belongs to. EDGE_TAPER_SEGMENTS is the polygon
// resolution along the spindle; higher reads smoother but costs more fill
// work per edge per frame.
const EDGE_TAPER_SEGMENTS = 8;
const EDGE_MIN_WIDTH_FACTOR = 0.3;
// Gravity: a soft per-node spring toward the origin (weak relative to the
// -150 charge/0.5 link forces, so it shapes the resting layout without
// fighting clustering) plus a hard position clamp as an absolute backstop —
// see the simulation setup below for why both exist.
const GRAVITY_STRENGTH = 0.03;
const MAX_RADIUS_FROM_CENTER = 600;

// Half-width of the edge spindle at parameter t along its length (0 = start
// node, 0.5 = midpoint, 1 = end node) — widest approaching either endpoint,
// narrowest at the midpoint.
function edgeHalfWidth(t: number, maxHalf: number) {
  const towardNode = Math.abs(2 * t - 1);
  return maxHalf * (EDGE_MIN_WIDTH_FACTOR + (1 - EDGE_MIN_WIDTH_FACTOR) * towardNode);
}

// Fills a tapered polygon between (sx, sy) and (tx, ty) instead of stroking
// a uniform-width line — Canvas 2D has no built-in variable-width stroke, so
// the spindle is built as a strip of quads along the segment's perpendicular.
function drawTaperedLink(
  ctx: CanvasRenderingContext2D,
  sx: number,
  sy: number,
  tx: number,
  ty: number,
  maxWidth: number,
) {
  const dx = tx - sx;
  const dy = ty - sy;
  const len = Math.hypot(dx, dy) || 1;
  const nx = -dy / len;
  const ny = dx / len;
  const maxHalf = maxWidth / 2;
  const top: Array<[number, number]> = [];
  const bottom: Array<[number, number]> = [];
  for (let i = 0; i <= EDGE_TAPER_SEGMENTS; i++) {
    const t = i / EDGE_TAPER_SEGMENTS;
    const x = sx + dx * t;
    const y = sy + dy * t;
    const half = edgeHalfWidth(t, maxHalf);
    top.push([x + nx * half, y + ny * half]);
    bottom.push([x - nx * half, y - ny * half]);
  }
  ctx.beginPath();
  ctx.moveTo(top[0][0], top[0][1]);
  for (let i = 1; i < top.length; i++) ctx.lineTo(top[i][0], top[i][1]);
  for (let i = bottom.length - 1; i >= 0; i--) ctx.lineTo(bottom[i][0], bottom[i][1]);
  ctx.closePath();
  ctx.fill();
}

/**
 * Generic force-directed map: nodes sized by a chosen metric, linked by a
 * chosen metric, with hover-to-highlight, drag-to-reposition,
 * click-to-select (via `onSelect`), wheel/pinch/button zoom, and
 * drag-the-background-to-pan. Fetches live data from `fetchUrl` and falls
 * back to a representative static graph if that fails. Shared by the tag
 * map and app map on the Explore page so the pan/zoom/drag/hover/resize/
 * accessibility plumbing lives in exactly one place.
 */
export function ForceMap<RawNode, RawLink>({
  fetchUrl,
  mapNode,
  mapLink,
  fallbackNodes,
  fallbackLinks,
  sizeMetrics,
  linkMetrics,
  ariaLabel,
  sourceLabel,
  onSelect,
  emptyMessage,
  selectRequest,
}: ForceMapProps<RawNode, RawLink>) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [source, setSource] = useState<"live" | "sample">("sample");
  const [loading, setLoading] = useState(true);
  const [isEmpty, setIsEmpty] = useState(false);
  const [hovered, setHovered] = useState<MapNode | null>(null);
  // Hover-to-highlight and drag are pointer-only (replicating them via
  // keyboard would need a whole separate nav model for what's a
  // supplementary view) — this sr-only list is the WCAG text alternative,
  // giving screen reader/keyboard users the same underlying data directly,
  // including the same select behavior as clicking a node on the canvas.
  const [nodeList, setNodeList] = useState<MapNode[]>(fallbackNodes);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showCustomize, setShowCustomize] = useState(false);
  const [sizeKey, setSizeKey] = useState(sizeMetrics[0].key);
  const [linkKey, setLinkKey] = useState(linkMetrics[0].key);
  // The fetch effect below only closes over `sizeKey`/`linkKey` as they were
  // at mount (its deps are [fetchUrl], intentionally, so a metric change
  // doesn't re-fetch/re-mount the simulation) — these refs let a metric
  // picked *while the fetch is still in flight* still apply once `start()`
  // finally runs, instead of the simulation silently initializing with the
  // stale mount-time default while the UI already shows the new selection.
  const sizeKeyRef = useRef(sizeKey);
  const linkKeyRef = useRef(linkKey);
  const applyMetricsRef = useRef<((size?: string, link?: string) => void) | undefined>(undefined);
  const zoomActionsRef = useRef<{ zoomIn: () => void; zoomOut: () => void; reset: () => void } | undefined>(
    undefined,
  );
  // Set once the simulation is running (see `start()` below) to the
  // closure's own `selectNode` — lets `selectRequest` (an external,
  // React-visible prop) reach into state that otherwise lives only inside
  // that closure, the same indirection `zoomActionsRef`/`applyMetricsRef`
  // already use for their own imperative actions.
  const externalSelectRef = useRef<((id: string | null) => void) | undefined>(undefined);
  // Exposes the same adjacency map the canvas click handler uses, so the
  // sr-only list's select action (keyboard/screen-reader path) reports the
  // same "connected peers" a mouse click would, instead of an empty array.
  const neighborsRef = useRef<Map<string, Set<string>>>(new Map());
  // Always the latest `selectRequest` prop, readable from inside `start()`
  // (a separate closure that only re-runs on `fetchUrl` changes — see that
  // effect's own deps comment). Lets a `selectRequest` already set BEFORE
  // the simulation finishes loading (e.g. one seeded from a URL query param
  // at mount, landing well before the live fetch resolves) still apply the
  // instant `start()` sets up `externalSelectRef` below, instead of being
  // silently dropped — see the effect below `start()`'s definition for why
  // that's otherwise a real gap, not just a theoretical one.
  const pendingSelectRequestRef = useRef(selectRequest);
  pendingSelectRequestRef.current = selectRequest;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setIsEmpty(false);
    // A previous fetchUrl's simulation may have left its last-drawn frame
    // sitting in the canvas's pixel buffer — its own rAF loop already got
    // cancelled by the cleanup below, so nothing will repaint over it on
    // its own. Clear it up front so neither the "Loading…" text nor an
    // empty-result message ever appears overlaid on stale nodes/edges.
    const canvasAtStart = canvasRef.current;
    canvasAtStart?.getContext("2d")?.clearRect(0, 0, canvasAtStart.width, canvasAtStart.height);
    fetch(fetchUrl)
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      // API responses are wrapped as { ok: true, data } by src/lib/api.ts's ok().
      .then((body: { data?: { nodes?: RawNode[]; edges?: RawLink[] } }) => {
        if (cancelled) return;
        const data = body.data;
        if (!data?.nodes) {
          // Malformed/missing response shape — treat like a fetch failure.
          setLoading(false);
          start(fallbackNodes, fallbackLinks);
          return;
        }
        if (data.nodes.length === 0) {
          // A live fetch that genuinely has nothing to show (e.g. a tag
          // filter combination matching no apps) is NOT a failure — showing
          // unrelated sample nodes here would silently misrepresent the
          // current filter as having real matches.
          setSource("live");
          setLoading(false);
          setIsEmpty(true);
          setNodeList([]);
          return;
        }
        setSource("live");
        setLoading(false);
        start(data.nodes.map(mapNode), (data.edges ?? []).map(mapLink));
      })
      .catch(() => {
        if (!cancelled) {
          setLoading(false);
          start(fallbackNodes, fallbackLinks);
        }
      });

    let cleanup: (() => void) | undefined;

    function start(rawNodes: MapNode[], rawLinks: MapLink[]) {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const nodes: MapNode[] = rawNodes.map((n) => ({ ...n }));
      const nodeById = new Map(nodes.map((n) => [n.id, n]));
      const links: MapLink[] = rawLinks
        .filter((l) => nodeById.has(String(l.source)) && nodeById.has(String(l.target)))
        .map((l) => ({ ...l }));

      let activeSizeKey = sizeKeyRef.current;
      let activeLinkKey = linkKeyRef.current;

      const sortKey = sizeMetrics[0].key;
      setNodeList([...rawNodes].sort((a, b) => (b.metrics[sortKey] ?? 0) - (a.metrics[sortKey] ?? 0)));

      // Direct-neighbor lookup for hover-highlight, selection-highlight, and
      // the "connected peers" list passed to onSelect — built once per
      // dataset rather than walked on every pointer move/click.
      const neighbors = new Map<string, Set<string>>();
      for (const n of nodes) neighbors.set(n.id, new Set());
      for (const l of links) {
        const s = String(l.source);
        const t = String(l.target);
        neighbors.get(s)?.add(t);
        neighbors.get(t)?.add(s);
      }
      neighborsRef.current = neighbors;

      let maxSize = Math.max(1, ...nodes.map((n) => n.metrics[activeSizeKey] ?? 0));
      const radius = (n: MapNode) =>
        MIN_RADIUS + Math.sqrt((n.metrics[activeSizeKey] ?? 0) / maxSize) * RADIUS_RANGE;

      let maxLink = Math.max(1, ...links.map((l) => l.metrics[activeLinkKey] ?? 0));
      const distance = (l: MapLink) => {
        const w = (l.metrics[activeLinkKey] ?? 0) / maxLink;
        return 96 - Math.min(56, w * 56);
      };

      let width = 0;
      let height = 0;
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      let hoveredNode: MapNode | null = null;
      let selectedNode: MapNode | null = null;

      // Pan/zoom view transform: node/simulation coordinates ("world" space)
      // stay centered on (0, 0) regardless of pan — only this transform
      // decides where that appears on screen, and by how much it's scaled.
      // Kept in a plain object (not React state) since it changes on every
      // wheel tick/pointer move and is only ever read by the rAF paint loop
      // and pointer-position math below, never by JSX.
      //
      // `view` is what's actually drawn; `viewTarget` is where it's headed.
      // A drag/wheel gesture writes both at once (1:1 tracking — a gesture
      // must never lag behind the pointer). A button click (zoom in/out/
      // reset) only moves `viewTarget`; the ease step in paintLoop below
      // chases `view` toward it every frame, so those clicks animate
      // smoothly instead of snapping the camera.
      const view = { k: 1, x: 0, y: 0 };
      const viewTarget = { k: 1, x: 0, y: 0 };
      // Time constant (seconds) for the view-follows-target ease — small
      // enough to feel immediate, large enough to actually read as motion
      // rather than a snap.
      const VIEW_EASE_TIME_CONSTANT = 0.15;
      let transformInitialized = false;

      function resize() {
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        width = rect.width;
        height = rect.height;
        canvas.width = Math.max(1, Math.floor(width * dpr));
        canvas.height = Math.max(1, Math.floor(height * dpr));
        simulation.force("center", forceCenter(0, 0));
        if (!transformInitialized) {
          // Only on first layout — center the world origin on screen. Later
          // resizes (e.g. a window resize) must not clobber a user's pan/zoom.
          view.x = width / 2;
          view.y = height / 2;
          viewTarget.x = view.x;
          viewTarget.y = view.y;
          transformInitialized = true;
        }
        simulation.alpha(0.6).restart();
      }

      const linkForce = forceLink<MapNode, MapLink>(links).id((d) => d.id).distance(distance).strength(0.5);
      const collideForce = forceCollide<MapNode>((n) => radius(n) + 10);

      const simulation = forceSimulation(nodes)
        .force("charge", forceManyBody().strength(-150))
        .force("link", linkForce)
        .force("collide", collideForce)
        .force("center", forceCenter(0, 0))
        // A soft, per-node spring toward the origin — unlike forceCenter
        // (which only nudges the whole layout so its AVERAGE position sits
        // at the origin, doing nothing for an individual outlier), this
        // pulls every node back toward the middle on its own, so a node
        // with few/no links can't drift away just because charge repulsion
        // pushed it there. Weak enough not to fight clustering.
        .force("gravityX", forceX(0).strength(GRAVITY_STRENGTH))
        .force("gravityY", forceY(0).strength(GRAVITY_STRENGTH))
        .alphaDecay(reduceMotion ? 1 : 0.02);

      // Hard backstop on top of the soft gravity springs above: a spring
      // pull always leaves SOME force balance where a node could still
      // rest very far out (e.g. an isolated node with no links, fighting
      // strong charge repulsion) — this guarantees no node ever renders
      // farther than MAX_RADIUS_FROM_CENTER from the origin, full stop.
      simulation.on("tick", () => {
        for (const n of nodes) {
          if (n.x == null || n.y == null) continue;
          const dist = Math.hypot(n.x, n.y);
          if (dist > MAX_RADIUS_FROM_CENTER) {
            const scale = MAX_RADIUS_FROM_CENTER / dist;
            n.x *= scale;
            n.y *= scale;
          }
        }
      });

      // Re-evaluates radius/distance against the currently selected metrics
      // and re-triggers d3-force's internal recompute (calling .distance()/
      // .radius() again forces it, even with the same function reference) —
      // lets the "Customize" controls retarget a running simulation without
      // re-fetching or re-mounting anything.
      applyMetricsRef.current = (nextSize, nextLink) => {
        if (nextSize) activeSizeKey = nextSize;
        if (nextLink) activeLinkKey = nextLink;
        maxSize = Math.max(1, ...nodes.map((n) => n.metrics[activeSizeKey] ?? 0));
        maxLink = Math.max(1, ...links.map((l) => l.metrics[activeLinkKey] ?? 0));
        linkForce.distance(distance);
        collideForce.radius((n) => radius(n) + 10);
        simulation.alpha(0.7).restart();
        // Keep the sr-only list's ranking in sync with whatever "Size by"
        // is currently selected, not pinned to the metric active at load.
        if (nextSize) {
          setNodeList([...rawNodes].sort((a, b) => (b.metrics[nextSize] ?? 0) - (a.metrics[nextSize] ?? 0)));
        }
      };

      // Zooms so that the point currently under `screenPoint` stays fixed on
      // screen — the standard "zoom to cursor" formula, clamped to
      // [MIN_ZOOM, MAX_ZOOM]. Used by the wheel handler, which is a live
      // gesture: it writes `view` directly (no easing lag) and keeps
      // `viewTarget` in lockstep so a zoom button clicked right after isn't
      // animating from a stale pre-gesture target.
      function zoomAt(screenPoint: { x: number; y: number }, factor: number) {
        const nextK = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.k * factor));
        const applied = nextK / view.k;
        view.x = screenPoint.x - (screenPoint.x - view.x) * applied;
        view.y = screenPoint.y - (screenPoint.y - view.y) * applied;
        view.k = nextK;
        viewTarget.k = view.k;
        viewTarget.x = view.x;
        viewTarget.y = view.y;
      }

      // Same "zoom to point" math, but for the +/-/reset buttons: a discrete
      // click isn't a gesture, so it only moves `viewTarget` — the ease step
      // in paintLoop below animates `view` to catch up, instead of the
      // camera jump-cutting to the new zoom level.
      function animateZoomAt(screenPoint: { x: number; y: number }, factor: number) {
        const nextK = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, viewTarget.k * factor));
        const applied = nextK / viewTarget.k;
        viewTarget.x = screenPoint.x - (screenPoint.x - viewTarget.x) * applied;
        viewTarget.y = screenPoint.y - (screenPoint.y - viewTarget.y) * applied;
        viewTarget.k = nextK;
      }

      zoomActionsRef.current = {
        zoomIn: () => animateZoomAt({ x: width / 2, y: height / 2 }, ZOOM_BUTTON_FACTOR),
        zoomOut: () => animateZoomAt({ x: width / 2, y: height / 2 }, 1 / ZOOM_BUTTON_FACTOR),
        reset: () => {
          viewTarget.k = 1;
          viewTarget.x = width / 2;
          viewTarget.y = height / 2;
        },
      };

      function isDimmed(id: string) {
        // Hover takes priority over selection while active; otherwise a
        // selection alone highlights itself + its neighbors the same way.
        const focus = hoveredNode ?? selectedNode;
        if (!focus) return false;
        return id !== focus.id && !neighbors.get(focus.id)?.has(id);
      }

      function draw() {
        if (!ctx) return;
        // Device-pixel-ratio correction is the base transform every frame;
        // pan/zoom is layered on top via translate/scale below, so none of
        // the node/link drawing code needs to know about either.
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, width, height);
        ctx.save();
        ctx.translate(view.x, view.y);
        ctx.scale(view.k, view.k);

        for (const l of links) {
          const s = l.source as MapNode;
          const t = l.target as MapNode;
          if (s.x == null || t.x == null) continue;
          const dim = isDimmed(s.id) || isDimmed(t.id);
          const focus = hoveredNode ?? selectedNode;
          const highlighted = focus != null && (s.id === focus.id || t.id === focus.id);
          const w = (l.metrics[activeLinkKey] ?? 0) / maxLink;
          ctx.fillStyle = EDGE_STROKE;
          ctx.globalAlpha = dim ? 0.05 : highlighted ? 0.5 : 0.15 + Math.min(0.25, w * 0.4);
          const maxWidth = (highlighted ? 3.2 : 1.8) / view.k;
          drawTaperedLink(ctx, s.x, s.y ?? 0, t.x, t.y ?? 0, maxWidth);
        }
        ctx.globalAlpha = 1;

        for (const n of nodes) {
          if (n.x == null || n.y == null) continue;
          const r = radius(n);
          const dim = isDimmed(n.id);
          const isHovered = hoveredNode?.id === n.id;
          const isSelected = selectedNode?.id === n.id;

          ctx.globalAlpha = dim ? 0.35 : 1;
          // A soft outward pulse on the selected node's glow — a slow
          // breathing beacon, not a distraction, and skipped entirely under
          // prefers-reduced-motion (reduceMotion freezes the sim too).
          const pulse =
            isSelected && !reduceMotion ? 1 + 0.12 * Math.sin(performance.now() * 0.003) : 1;
          const grad = ctx.createRadialGradient(n.x, n.y, 0, n.x, n.y, r * 2 * pulse);
          grad.addColorStop(0, dim ? "rgba(148, 151, 163, 0.18)" : "rgba(99, 102, 241, 0.3)");
          grad.addColorStop(1, "rgba(99, 102, 241, 0)");
          ctx.fillStyle = grad;
          ctx.beginPath();
          ctx.arc(n.x, n.y, r * 2 * pulse, 0, Math.PI * 2);
          ctx.fill();

          ctx.fillStyle = dim ? NODE_FILL_DIM : NODE_FILL;
          ctx.beginPath();
          ctx.arc(n.x, n.y, isHovered ? r * 0.55 : r * 0.4, 0, Math.PI * 2);
          ctx.fill();
          if (isSelected) {
            ctx.strokeStyle = SELECTED_RING;
            ctx.lineWidth = 2.5 / view.k;
            ctx.beginPath();
            ctx.arc(n.x, n.y, (r * 0.4 + 4) * pulse / view.k, 0, Math.PI * 2);
            ctx.stroke();
          } else if (isHovered) {
            ctx.strokeStyle = LABEL_INK;
            ctx.lineWidth = 2 / view.k;
            ctx.stroke();
          }

          const fontSize = isHovered ? 12 : 11;
          ctx.font = isHovered
            ? `600 ${fontSize}px ui-sans-serif, system-ui, sans-serif`
            : `${fontSize}px ui-sans-serif, system-ui, sans-serif`;
          ctx.fillStyle = dim ? LABEL_DIM : LABEL_INK;
          ctx.textAlign = "center";
          ctx.fillText(n.label, n.x, n.y - r - 6);
        }
        ctx.globalAlpha = 1;
        ctx.restore();
      }

      // The simulation runs its own internal timer regardless of listeners;
      // this rAF loop just repaints from its latest node positions, for the
      // lifetime of the component. Paused while the canvas is off-screen.
      let raf = 0;
      let stopped = false;
      let onScreen = true;
      let lastFrameTime = performance.now();
      function paintLoop() {
        if (stopped) return;
        const now = performance.now();
        // Clamp dt so a stalled/backgrounded tab doesn't resume with one huge
        // catch-up jump — worst case it just takes one extra frame to settle.
        const dt = Math.min(0.1, (now - lastFrameTime) / 1000);
        lastFrameTime = now;
        if (onScreen) {
          if (reduceMotion) {
            // No moving camera under reduced motion — button zoom/reset just
            // takes effect immediately instead of animating there.
            view.k = viewTarget.k;
            view.x = viewTarget.x;
            view.y = viewTarget.y;
          } else {
            const ease = 1 - Math.exp(-dt / VIEW_EASE_TIME_CONSTANT);
            view.k += (viewTarget.k - view.k) * ease;
            view.x += (viewTarget.x - view.x) * ease;
            view.y += (viewTarget.y - view.y) * ease;
          }
          draw();
        }
        raf = requestAnimationFrame(paintLoop);
      }
      raf = requestAnimationFrame(paintLoop);

      const io = new IntersectionObserver((entries) => {
        onScreen = entries[0]?.isIntersecting ?? true;
      });
      io.observe(canvas);

      const ro = new ResizeObserver(resize);
      ro.observe(canvas);
      resize();

      function screenPos(e: PointerEvent | WheelEvent) {
        const rect = canvas!.getBoundingClientRect();
        return { x: e.clientX - rect.left, y: e.clientY - rect.top };
      }
      function toWorld(p: { x: number; y: number }) {
        return { x: (p.x - view.x) / view.k, y: (p.y - view.y) / view.k };
      }
      function nodeAt(world: { x: number; y: number }): MapNode | null {
        let closest: MapNode | null = null;
        let closestDist = Infinity;
        for (const n of nodes) {
          if (n.x == null || n.y == null) continue;
          const dist = Math.hypot(n.x - world.x, n.y - world.y);
          // Hit-radius padding is generous and NOT divided by zoom, so
          // zoomed-out nodes (and touch targets in general) stay easy to
          // hit rather than shrinking to unusable pixel sizes.
          if (dist < radius(n) + 10 / view.k && dist < closestDist) {
            closest = n;
            closestDist = dist;
          }
        }
        return closest;
      }

      function selectNode(node: MapNode | null) {
        selectedNode = node;
        setSelectedId(node?.id ?? null);
        onSelect?.(node, node ? [...(neighbors.get(node.id) ?? [])] : []);
      }

      externalSelectRef.current = (id) => {
        selectNode(id ? nodes.find((n) => n.id === id) ?? null : null);
      };

      // A `selectRequest` set before the simulation was ready (see
      // `pendingSelectRequestRef`'s doc comment) is applied now, the instant
      // it's actually possible to — the effect below only re-fires on a
      // *change* to `selectRequest`, which never happens for a value that
      // was already there at mount.
      if (pendingSelectRequestRef.current) {
        externalSelectRef.current(pendingSelectRequestRef.current.id);
      }

      // Wheel = zoom to cursor. Listened natively (not React's onWheel) so
      // preventDefault reliably stops page scroll while the cursor is over
      // the map, which requires an explicit non-passive listener.
      function onWheel(e: WheelEvent) {
        e.preventDefault();
        const factor = Math.exp(-e.deltaY * 0.0015);
        zoomAt(screenPos(e), factor);
      }

      // Drag-to-reposition a node, OR drag-the-background-to-pan — decided
      // by whether the pointer went down on a node. Listeners live on the
      // canvas itself, not window: setPointerCapture below routes every
      // subsequent event for this pointerId to the canvas regardless of
      // where the cursor physically is, so a window-level listener isn't
      // needed for either gesture to keep working once the cursor leaves
      // the canvas mid-drag.
      let draggingNode: MapNode | null = null;
      let panning = false;
      let downPos: { x: number; y: number } | null = null;
      let panOrigin = { x: 0, y: 0 };
      function onDown(e: PointerEvent) {
        const p = screenPos(e);
        downPos = p;
        const hit = nodeAt(toWorld(p));
        if (hit) {
          draggingNode = hit;
          draggingNode.fx = draggingNode.x;
          draggingNode.fy = draggingNode.y;
          simulation.alphaTarget(0.3).restart();
        } else {
          panning = true;
          panOrigin = { x: view.x, y: view.y };
        }
        canvas!.setPointerCapture(e.pointerId);
      }
      function onMove(e: PointerEvent) {
        const p = screenPos(e);
        if (draggingNode) {
          const w = toWorld(p);
          draggingNode.fx = w.x;
          draggingNode.fy = w.y;
          return;
        }
        if (panning && downPos) {
          view.x = panOrigin.x + (p.x - downPos.x);
          view.y = panOrigin.y + (p.y - downPos.y);
          viewTarget.x = view.x;
          viewTarget.y = view.y;
          return;
        }
        const hit = nodeAt(toWorld(p));
        if (hit?.id !== hoveredNode?.id) {
          hoveredNode = hit;
          setHovered(hit);
        }
        canvas!.style.cursor = hit ? (onSelect ? "pointer" : "grab") : "default";
      }
      function onUp(e: PointerEvent) {
        const moved = downPos ? Math.hypot(screenPos(e).x - downPos.x, screenPos(e).y - downPos.y) : Infinity;
        const wasClick = moved < CLICK_DRAG_THRESHOLD_PX;

        if (draggingNode) {
          const node = draggingNode;
          draggingNode.fx = null;
          draggingNode.fy = null;
          simulation.alphaTarget(0);
          draggingNode = null;
          if (onSelect && wasClick) {
            selectNode(selectedNode?.id === node.id ? null : node);
          }
        } else if (panning && wasClick && onSelect) {
          // A background "click" (not a real pan) with something already
          // selected clears it — same as clicking empty space in any other
          // selectable UI.
          selectNode(null);
        }
        panning = false;
        downPos = null;
      }
      function onLeave() {
        hoveredNode = null;
        setHovered(null);
      }
      canvas.addEventListener("pointerdown", onDown);
      canvas.addEventListener("pointerleave", onLeave);
      canvas.addEventListener("pointermove", onMove);
      canvas.addEventListener("pointerup", onUp);
      canvas.addEventListener("wheel", onWheel, { passive: false });

      cleanup = () => {
        stopped = true;
        cancelAnimationFrame(raf);
        simulation.stop();
        io.disconnect();
        ro.disconnect();
        applyMetricsRef.current = undefined;
        zoomActionsRef.current = undefined;
        externalSelectRef.current = undefined;
        neighborsRef.current = new Map();
        canvas!.removeEventListener("pointerdown", onDown);
        canvas!.removeEventListener("pointerleave", onLeave);
        canvas!.removeEventListener("pointermove", onMove);
        canvas!.removeEventListener("pointerup", onUp);
        canvas!.removeEventListener("wheel", onWheel);
      };
    }

    return () => {
      cancelled = true;
      cleanup?.();
    };
    // fetchUrl is the only prop that should ever re-run the fetch/simulation
    // setup; metric selection is applied imperatively via applyMetricsRef so
    // toggling it doesn't re-fetch or restart the drag/hover listeners.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fetchUrl]);

  // `selectRequest` going from null to a real request always means "select
  // this," even if the caller re-selects the same id twice in a row — see
  // `selectRequest`'s doc comment on `ForceMapProps` for why that needs a
  // fresh object reference rather than an id-equality check to notice.
  // No-ops harmlessly if the simulation hasn't started yet (externalSelectRef
  // still unset) — covered separately by `pendingSelectRequestRef` above,
  // which is what actually applies a request that was already set at mount
  // (before the simulation had a chance to start) once it does.
  useEffect(() => {
    if (selectRequest) externalSelectRef.current?.(selectRequest.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectRequest]);

  function handleSizeChange(key: string) {
    setSizeKey(key);
    sizeKeyRef.current = key;
    applyMetricsRef.current?.(key, undefined);
  }
  function handleLinkChange(key: string) {
    setLinkKey(key);
    linkKeyRef.current = key;
    applyMetricsRef.current?.(undefined, key);
  }
  function handleNodeListSelect(n: MapNode) {
    const deselecting = selectedId === n.id;
    onSelect?.(deselecting ? null : n, deselecting ? [] : [...(neighborsRef.current.get(n.id) ?? [])]);
    setSelectedId(deselecting ? null : n.id);
  }

  const activeSizeMetric = sizeMetrics.find((m) => m.key === sizeKey) ?? sizeMetrics[0];
  const hasOptions = sizeMetrics.length > 1 || linkMetrics.length > 1;
  const customizePanelId = useId();

  return (
    <div>
      <div className="relative">
        <canvas
          ref={canvasRef}
          className="explore-map-surface block h-[24rem] w-full touch-none rounded-card border border-hairline sm:h-[30rem]"
          role="img"
          aria-label={ariaLabel}
        />
        {isEmpty && (
          <div className="pointer-events-none absolute inset-0 grid place-items-center px-6 text-center">
            <p className="text-sm text-slate-steel">{emptyMessage ?? "Nothing to show."}</p>
          </div>
        )}
        {loading && !isEmpty && (
          <div className="pointer-events-none absolute inset-0 grid place-items-center">
            <p className="text-sm text-slate-steel">Loading…</p>
          </div>
        )}
        {hovered && (
          <div className="pointer-events-none absolute left-3 top-3 max-w-[14rem] rounded-card border border-hairline bg-cream/90 px-3 py-2 backdrop-blur-sm">
            <div className="text-sm font-semibold text-ink">{hovered.label}</div>
            {sizeMetrics.map((m) => (
              <div key={m.key} className="text-caption text-slate">
                {(m.format ?? String)(hovered.metrics[m.key] ?? 0)}
              </div>
            ))}
          </div>
        )}
        <div className="absolute bottom-3 right-3 flex flex-col overflow-hidden rounded-card border border-hairline bg-cream/80 backdrop-blur-sm">
          <button
            type="button"
            onClick={() => zoomActionsRef.current?.zoomIn()}
            aria-label="Zoom in"
            className="grid h-10 w-10 place-items-center text-slate transition-colors duration-150 hover:bg-mist"
          >
            +
          </button>
          <div className="h-px bg-hairline" />
          <button
            type="button"
            onClick={() => zoomActionsRef.current?.zoomOut()}
            aria-label="Zoom out"
            className="grid h-10 w-10 place-items-center text-slate transition-colors duration-150 hover:bg-mist"
          >
            −
          </button>
          <div className="h-px bg-hairline" />
          <button
            type="button"
            onClick={() => zoomActionsRef.current?.reset()}
            aria-label="Reset zoom and pan"
            className="grid h-10 w-10 place-items-center text-sm text-slate transition-colors duration-150 hover:bg-mist"
          >
            ⟲
          </button>
        </div>
      </div>
      <ul className="sr-only">
        {nodeList.map((n) => {
          const summary = `${n.label}: ${sizeMetrics.map((m) => (m.format ?? String)(n.metrics[m.key] ?? 0)).join(", ")}`;
          return (
            <li key={n.id}>
              {onSelect ? (
                <button type="button" onClick={() => handleNodeListSelect(n)}>
                  {summary}
                </button>
              ) : (
                summary
              )}
            </li>
          );
        })}
      </ul>
      <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-caption text-slate-steel">
        <span>
          {source === "live" ? `Live ${sourceLabel}.` : `Sample ${sourceLabel}.`}{" "}
          {onSelect
            ? "Drag a node to reposition, drag the background to pan, scroll to zoom, click to select."
            : "Drag to rearrange, hover to see connections."}
        </span>
        <span className="flex items-center gap-3">
          <span className="flex items-center gap-1.5">
            <span className="inline-block h-2 w-2 rounded-full bg-cobalt" aria-hidden="true" />
            size = {activeSizeMetric.label.toLowerCase()}
          </span>
          {hasOptions && (
            <button
              type="button"
              onClick={() => setShowCustomize((v) => !v)}
              aria-expanded={showCustomize}
              aria-controls={customizePanelId}
              className="font-medium text-cobalt hover:text-cobalt-deep hover:underline"
            >
              {showCustomize ? "Hide options" : "Customize"}
            </button>
          )}
        </span>
      </div>
      {hasOptions && showCustomize && (
        <div id={customizePanelId} className="mt-3 flex flex-wrap gap-4 rounded-card border border-hairline bg-cream/80 p-3 backdrop-blur-sm">
          {linkMetrics.length > 1 && (
            <div>
              <div className="text-caption font-semibold uppercase tracking-wide text-slate-steel">Connect by</div>
              <div className="mt-1.5 flex flex-wrap gap-1.5">
                {linkMetrics.map((m) => (
                  <button
                    key={m.key}
                    type="button"
                    onClick={() => handleLinkChange(m.key)}
                    aria-pressed={linkKey === m.key}
                    className={cn(DARK_CHIP, linkKey === m.key && DARK_CHIP_ACTIVE)}
                  >
                    {m.label}
                  </button>
                ))}
              </div>
            </div>
          )}
          {sizeMetrics.length > 1 && (
            <div>
              <div className="text-caption font-semibold uppercase tracking-wide text-slate-steel">Size by</div>
              <div className="mt-1.5 flex flex-wrap gap-1.5">
                {sizeMetrics.map((m) => (
                  <button
                    key={m.key}
                    type="button"
                    onClick={() => handleSizeChange(m.key)}
                    aria-pressed={sizeKey === m.key}
                    className={cn(DARK_CHIP, sizeKey === m.key && DARK_CHIP_ACTIVE)}
                  >
                    {m.label}
                  </button>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
