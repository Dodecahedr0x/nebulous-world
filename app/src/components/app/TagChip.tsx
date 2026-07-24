"use client";

import { useRouter } from "next/navigation";
import type { TagDTO } from "@/lib/types";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { cn, formatToken } from "@/lib/utils";

/**
 * A tag pill that deep-links into Rankings' Map view, Tags tab, with this
 * tag preselected/highlighted on the map (`/rankings?view=map&tab=tags&
 * tags=<slug>` — see RankingsTabs and ExploreMaps, which both read their
 * active tab/selection from the URL for exactly this, and ForceMap's
 * `selectRequest` doc comment for how a URL-driven selection lands even
 * though the map's own force simulation is still loading at that point):
 * clicking a tag reads as "show me more like this," the same expectation a
 * tag/topic chip carries anywhere else, not as a shortcut into committing
 * NEB to one specific app's stake panel.
 *
 * A `button`, not a `Link` — AppCard already wraps the whole card in a
 * `<Link>` to the app page, and nesting an `<a>` inside an `<a>` is invalid;
 * hence the stopPropagation pattern below.
 */
export function TagChip({ tag, className }: { tag: TagDTO; className?: string }) {
  const router = useRouter();
  return (
    <button
      type="button"
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        router.push(`/rankings?view=map&tab=tags&tags=${encodeURIComponent(tag.slug)}`);
      }}
      className={cn(
        "chip text-[11px] transition-colors duration-150 hover:border-cobalt/50 hover:text-cobalt",
        tag.stakeTotal > 0 && "chip-active",
        className,
      )}
      title={
        tag.stakeTotal > 0
          ? `${formatToken(tag.stakeTotal, TOKEN_SYMBOL)} staked — view all #${tag.name} apps`
          : `View all #${tag.name} apps`
      }
    >
      #{tag.name}
      {tag.stakeTotal > 0 && (
        <span className="text-cobalt">{formatToken(tag.stakeTotal, "")}</span>
      )}
    </button>
  );
}
