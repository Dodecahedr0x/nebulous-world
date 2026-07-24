"use client";

import { useId } from "react";
import { AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer } from "recharts";
import { formatNumber, formatToken, splitValueUnit } from "@/lib/utils";
import { TOKEN_SYMBOL } from "@/lib/constants";

// DESIGN.md tokens (see tailwind.config.ts): ivory=surface, hairline=border,
// slate=ink muted, ink=primary text, cobalt=indigo — the light-theme values
// of the same named tokens this card has always read from.

export interface TrendPoint {
  x: string;
  y: number;
}

// A chart with fewer than 2 points can't draw a line at all. Rather than
// swap in a text fallback (which would make an empty-history card a
// different shape from its neighbors), pad up to this many flat zero
// points — a flat baseline at 0 reads as "nothing here yet" on its own,
// no caption needed, and every card keeps the same layout.
const MIN_POINTS = 7;
const ONE_DAY_MS = 24 * 60 * 60 * 1000;
const TICK_STYLE = { fontSize: 9, fill: "#565a66" };

// Real, evenly-day-spaced dates ending today rather than plain index
// strings ("0".."6") — the x-axis is a genuine time scale (see below), so
// every point on it needs a real, parseable date to plot correctly.
function withZeroFloor(data: TrendPoint[]): TrendPoint[] {
  if (data.length >= 2) return data;
  const now = Date.now();
  return Array.from({ length: MIN_POINTS }, (_, i) => ({
    x: new Date(now - (MIN_POINTS - 1 - i) * ONE_DAY_MS).toISOString(),
    y: 0,
  }));
}

// x is plotted on a numeric time scale (epoch ms — see the XAxis below),
// not laid out one-point-per-array-index: two points a week apart sit
// visibly farther apart than two points an hour apart, instead of both
// getting the same fixed-width slot regardless of the actual gap between
// them.
function toChartPoints(data: TrendPoint[]): { x: number; y: number }[] {
  return withZeroFloor(data).map((p) => ({ x: new Date(p.x).getTime(), y: p.y }));
}

// Compact "Jan 5" tick/tooltip date, from either an ISO string or the
// epoch-ms number recharts hands tickFormatter/the tooltip's `label`.
function formatTickDate(x: string | number): string {
  const d = new Date(x);
  return Number.isNaN(d.getTime()) ? String(x) : d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

// Deliberately still a dark-glass popover (unlike ForceMap/GroupMap's hover
// cards, migrated to light in an earlier task) — a small, high-contrast
// tooltip is a common pattern independent of page theme, similar to a
// native OS tooltip, and wasn't a redesign target for this component.
function ChartTooltip({
  active,
  payload,
  label,
  valueFormat,
}: {
  active?: boolean;
  payload?: { value: number }[];
  label?: string | number;
  valueFormat: (n: number) => string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="rounded-card border border-white/10 bg-black/80 px-2.5 py-1.5 backdrop-blur-sm">
      <div className="text-xs font-semibold text-white">{valueFormat(payload[0].value)}</div>
      <div className="text-[10px] text-white/50">{formatTickDate(label ?? "")}</div>
    </div>
  );
}

/**
 * One Explore-page metric tile: the current value plus a full-bleed filled
 * trend chart underneath (see DESIGN.md's Astro reference), with a compact
 * axis and a hover tooltip on the trend itself.
 */
export function MetricTrendCard({
  label,
  value,
  data,
  valueKind = "number",
}: {
  label: string;
  value: string;
  data: TrendPoint[];
  /** How to format a single point's raw y value for the hover tooltip —
      "token" for a NEB-denominated series (votes, stake), "number" (the
      default) for a plain count. A formatter function can't be passed here
      instead: this card is a Client Component rendered from the (async,
      Server Component) Explore page, and functions aren't serializable
      across that boundary. */
  valueKind?: "number" | "token";
}) {
  const gradientId = useId();
  const chartData = toChartPoints(data);
  const valueFormat = valueKind === "token" ? (n: number) => formatToken(n, TOKEN_SYMBOL) : formatNumber;
  // Split "1.23M NEB" -> ["1.23M", "NEB"] so the unit renders smaller and
  // muted next to the bolded figure, matching the reference's "123 Mil"
  // treatment. Values with no unit (plain formatNumber output) just render
  // as the one bolded span.
  const [amount, unit] = splitValueUnit(value);

  return (
    <div className="card flex flex-col justify-between">
      <div className="min-w-0 p-6 pb-0">
        <div className="text-caption font-semibold uppercase tracking-wide text-slate">
          {label}
        </div>
        {/* flex-wrap, not nowrap: a wide value (e.g. "133.96K NEB") wraps
            the unit to its own line instead of overflowing the card —
            clipping it against the chart wrapper's overflow-hidden below
            was the previous bug here. Value stays achromatic (text-ink,
            not a chromatic accent) and within the documented type scale at
            heading-sm — DESIGN.md's whole system spends color sparingly
            ("every chromatic pixel is doing real work"), and text-heading-xl
            here tied visually with the page's own H1 at the same size. The
            trend line below is the one deliberate accent per card instead. */}
        <div className="mt-1 flex flex-wrap items-baseline gap-x-1.5">
          <span className="text-heading-sm font-semibold tabular-nums text-ink">
            {amount}
          </span>
          {unit && (
            <span className="text-subheading font-medium text-slate">{unit}</span>
          )}
        </div>
      </div>
      {/* Bled flush to the card's own left/right/bottom edges (the
          reference's defining trait) — only the axis ticks themselves ask
          for a little internal margin, via the AreaChart's own `margin`
          prop below, rather than padding this wrapper. */}
      <div className="mt-4 h-24 overflow-hidden rounded-b-card">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={chartData} margin={{ top: 4, right: 6, bottom: 0, left: 0 }}>
            <defs>
              <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="#4338ca" stopOpacity={0.35} />
                <stop offset="100%" stopColor="#4338ca" stopOpacity={0} />
              </linearGradient>
            </defs>
            <XAxis
              dataKey="x"
              type="number"
              scale="time"
              domain={["dataMin", "dataMax"]}
              tickFormatter={formatTickDate}
              tick={TICK_STYLE}
              axisLine={false}
              tickLine={false}
              interval="preserveStartEnd"
              minTickGap={24}
            />
            <YAxis
              width={30}
              tick={TICK_STYLE}
              axisLine={false}
              tickLine={false}
              domain={["dataMin", "dataMax"]}
              tickFormatter={formatNumber}
            />
            <Tooltip
              content={<ChartTooltip valueFormat={valueFormat} />}
              cursor={{ stroke: "#4338ca", strokeOpacity: 0.3 }}
              isAnimationActive={false}
            />
            <Area
              type="monotone"
              dataKey="y"
              stroke="#4338ca"
              strokeWidth={2}
              fill={`url(#${gradientId})`}
              isAnimationActive={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
