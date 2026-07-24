import { clsx, type ClassValue } from "clsx";
import type { TagDTO } from "./types";

/** Tailwind-friendly className combiner. */
export function cn(...inputs: ClassValue[]): string {
  return clsx(inputs);
}

/** Convert an arbitrary string into a URL-safe slug. */
export function slugify(input: string): string {
  return input
    .toLowerCase()
    .trim()
    .replace(/['"]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80);
}

/** Shorten a base58 address for display: "9xQe…3kf2". */
export function shortAddress(addr: string, chars = 4): string {
  if (!addr) return "";
  if (addr.length <= chars * 2 + 1) return addr;
  return `${addr.slice(0, chars)}…${addr.slice(-chars)}`;
}

/** Format a token amount with thousands separators, never more than 2 decimals. */
export function formatToken(amount: number, symbol = "NEB"): string {
  const abs = Math.abs(amount);
  let str: string;
  if (abs >= 1_000_000) str = (amount / 1_000_000).toFixed(2) + "M";
  else if (abs >= 1_000) str = (amount / 1_000).toFixed(2) + "K";
  else str = amount.toFixed(2);
  return symbol ? `${str} ${symbol}` : str;
}

export function formatNumber(n: number): string {
  return new Intl.NumberFormat("en-US").format(n);
}

/** Split a formatted value like "1.23M NEB" into its figure and unit: ["1.23M", "NEB"]. */
export function splitValueUnit(value: string): [amount: string, unit: string] {
  const [amount, ...unitParts] = value.split(" ");
  return [amount, unitParts.join(" ")];
}

/** Extract a display-friendly hostname from a URL: "https://www.jup.ag/x" -> "jup.ag". */
export function hostname(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** Formats a stat's recent percent change for AppCard's subtext, e.g.
    "+12%/7d" or "-8%/30d" — `null` (rendered as nothing) when there's no
    baseline to compare against, not when the change happens to be small: a
    genuine 0% still shows as "+0%/7d" rather than being hidden, since that
    correctly communicates "flat," not "unknown." Rounded to the nearest
    whole percent — the subtext is a few characters under a stat number, not
    a place for decimal precision. */
export function formatDelta(pct: number | null, intervalDays: number): string | null {
  if (pct === null) return null;
  const rounded = Math.round(pct);
  const sign = rounded >= 0 ? "+" : "";
  return `${sign}${rounded}%/${intervalDays}d`;
}

/** The app's tag with the most stake behind it, or null if it has no tags
    at all — apps have no onchain "category", so this is what stands in for
    one anywhere a card previously showed `app.category`. */
export function topStakedTag(tags: TagDTO[]): TagDTO | null {
  if (tags.length === 0) return null;
  return tags.reduce((top, t) => (t.stakeTotal > top.stakeTotal ? t : top));
}

export function timeAgo(d: Date | string): string {
  const date = typeof d === "string" ? new Date(d) : d;
  const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
  const intervals: [number, string][] = [
    [31536000, "y"],
    [2592000, "mo"],
    [86400, "d"],
    [3600, "h"],
    [60, "m"],
  ];
  for (const [secs, label] of intervals) {
    const count = Math.floor(seconds / secs);
    if (count >= 1) return `${count}${label} ago`;
  }
  return "just now";
}
