import type { Metadata } from "next";
import Link from "next/link";
import { fetchPlatformStats } from "@/lib/indexerClient";
import { cn, formatNumber, formatToken, splitValueUnit } from "@/lib/utils";
import { SITE_NAME, SITE_URL, TOKEN_NAME, TOKEN_SYMBOL } from "@/lib/constants";
import { X402_ENDPOINTS } from "@/lib/x402";
import { ConstellationField } from "@/components/about/ConstellationField";

export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "About",
  description:
    "nebulous.world is crowd-sourced app discovery on Solana: transparent, token-weighted ranking, tag staking, and ad revenue shared with the people backing what's good.",
  alternates: { canonical: `${SITE_URL}/about` },
};

const FEATURES = [
  {
    color: "text-forest border-forest/40",
    title: "Ranked by the crowd, not a listing fee",
    body: "Every app's rank blends token-weighted votes, tag stake, and real traffic — no pay-to-play placement, no opaque algorithm. The math is open source.",
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M4 19V10M12 19V5M20 19v-7"
      />
    ),
  },
  {
    color: "text-cobalt border-cobalt/40",
    title: "Skin in the game",
    body: `Anyone can suggest a tag for an app; staking ${TOKEN_SYMBOL} behind one you believe in boosts its rank and puts your conviction on chain, not just an opinion in a review box.`,
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M7 7h.01M4 4h9.586a1 1 0 01.707.293l6 6a1 1 0 010 1.414l-8.586 8.586a1 1 0 01-1.414 0l-6-6A1 1 0 014 13.586V5a1 1 0 011-1z"
      />
    ),
  },
  {
    color: "text-violet border-violet/40",
    title: "Upside shared with stakers",
    body: "Ad revenue an app's page earns is distributed to everyone staked behind it — across all its tags — proportional to stake. The platform's growth is everyone's growth.",
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V6m0 10v2"
      />
    ),
  },
];

const STEPS = [
  {
    label: "Discover",
    body: "Search and filter by tag, chain, or category, sorted by a transparent rank score.",
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M21 21l-4.35-4.35M11 18a7 7 0 100-14 7 7 0 000 14z"
      />
    ),
  },
  {
    label: "Vote",
    body: `Commit ${TOKEN_SYMBOL} to an app you use. Votes are token-weighted and feed straight into its rank.`,
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M5 13l4 4L19 7"
      />
    ),
  },
  {
    label: "Stake a tag",
    body: "Back the tags that actually describe an app — stake boosts its rank and starts earning.",
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M7 20l4-16m2 16l4-16M6 9h14M4 15h14"
      />
    ),
  },
  {
    label: "Earn",
    body: "Claim your share of ad revenue anytime, without withdrawing your vote or stake.",
    icon: (
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M12 2v20m5-17H9.5a3.5 3.5 0 000 7h5a3.5 3.5 0 010 7H6"
      />
    ),
  },
];

export default async function AboutPage() {
  const stats = await fetchPlatformStats();

  return (
    <div className="space-y-24 pb-8">
      {/* Full-bleed hero — breaks out of AppShell's max-w-7xl container so
          the shader/gradient backdrop reaches the true viewport edges; the
          text content inside is re-constrained to the normal column. */}
      <section className="relative left-1/2 -mt-8 w-screen -translate-x-1/2 overflow-hidden">
        <ConstellationField className="absolute inset-0 -z-10 h-full w-full" />
        <div
          className="absolute inset-0 -z-10"
          style={{ background: "radial-gradient(circle at 50% 0%, rgba(67, 56, 202, 0.08) 0%, rgba(255, 255, 255, 0) 60%)" }}
        />
        <div className="mx-auto flex max-w-3xl flex-col items-center gap-6 px-4 py-32 text-center sm:px-6 lg:px-8">
          <span className="rounded-pill border border-white/15 bg-white/10 px-4 py-1.5 text-caption font-semibold uppercase tracking-wide text-cream/80 backdrop-blur-sm">
            Crowd-sourced app discovery on Solana
          </span>
          <h1 className="text-balance font-display text-heading-xl font-normal text-white sm:text-display sm:leading-[1.05]">
            Discover what the crowd already knows
          </h1>
          <p className="text-pretty text-subheading text-cream/70">
            {SITE_NAME} ranks apps by real conviction — token-weighted votes, tag stake, and
            traffic — then shares what they earn with the people who backed them first.
          </p>
          <div className="mt-2 flex flex-wrap items-center justify-center gap-3">
            <Link href="/" className="btn-primary">
              Browse apps
            </Link>
            <Link href="/explore" className="btn-secondary">
              See the map
            </Link>
          </div>
        </div>
      </section>

      {/* Why it's different */}
      <section className="reveal mx-auto max-w-6xl px-4 sm:px-6 lg:px-8">
        <div className="max-w-2xl">
          <span className="text-body-sm font-semibold uppercase tracking-wide text-forest">
            Why it&apos;s different
          </span>
          <h2 className="mt-2 text-balance font-display text-heading font-light text-ink">
            Ranking with real skin in the game
          </h2>
        </div>
        <div className="mt-10 grid gap-8 sm:grid-cols-3">
          {FEATURES.map((f) => (
            <div key={f.title}>
              <div
                className={`grid h-12 w-12 place-items-center rounded-full border ${f.color}`}
              >
                <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  {f.icon}
                </svg>
              </div>
              <h3 className="mt-4 font-display text-subheading font-normal text-ink">
                {f.title}
              </h3>
              <p className="mt-2 text-pretty text-body-sm text-slate">{f.body}</p>
            </div>
          ))}
        </div>
      </section>

      {/* How it works */}
      <section className="reveal mx-auto max-w-6xl px-4 sm:px-6 lg:px-8">
        <div className="max-w-2xl">
          <span className="text-body-sm font-semibold uppercase tracking-wide text-cobalt">
            How it works
          </span>
          <h2 className="mt-2 text-balance font-display text-heading font-light text-ink">
            Four steps, all of it on chain
          </h2>
        </div>
        <div className="relative mt-10 grid gap-8 sm:grid-cols-4">
          {/* Connecting line behind the step circles — desktop only, where
              the steps actually sit in a single row. */}
          <div
            className="absolute left-0 right-0 top-6 hidden h-px bg-hairline sm:block"
            aria-hidden="true"
          />
          {STEPS.map((step, i) => (
            <div key={step.label} className="relative flex flex-col items-start gap-3">
              <div className="grid h-12 w-12 place-items-center rounded-full border border-hairline bg-cream text-ink">
                <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  {step.icon}
                </svg>
              </div>
              <div>
                <div className="flex items-center gap-2">
                  <span className="text-caption font-semibold text-slate-steel">
                    {String(i + 1).padStart(2, "0")}
                  </span>
                  <h3 className="font-display text-body font-normal text-ink">{step.label}</h3>
                </div>
                <p className="mt-1 text-pretty text-body-sm text-slate">{step.body}</p>
              </div>
            </div>
          ))}
        </div>
      </section>

      {/* Data API — every dataset priced individually and paid for over
          x402, no signup. The pricing table below reads X402_ENDPOINTS
          directly (app/src/lib/x402.ts) — the same config the /api/data/*
          routes themselves enforce — so this can't drift out of sync with
          what a request actually costs. */}
      <section className="reveal mx-auto max-w-6xl px-4 sm:px-6 lg:px-8">
        <div className="max-w-2xl">
          <span className="text-body-sm font-semibold uppercase tracking-wide text-amber">
            Data API
          </span>
          <h2 className="mt-2 text-balance font-display text-heading font-light text-ink">
            Pay per query, no signup
          </h2>
          <p className="mt-3 text-pretty text-body-sm text-slate">
            Every dataset below is priced in {TOKEN_SYMBOL} and metered per request over{" "}
            <a
              href="https://www.x402.org"
              target="_blank"
              rel="noopener noreferrer"
              className="text-cobalt hover:underline"
            >
              x402
            </a>
            , the open HTTP-native micropayment standard: request the resource, get back
            an HTTP 402 with the exact price, pay in the same round trip, get the data.
            No API key, no account, no subscription — an autonomous agent (or a script)
            can start querying immediately.
          </p>
        </div>

        <div className="mt-10 grid gap-4 sm:grid-cols-2">
          {Object.values(X402_ENDPOINTS).map((endpoint) => (
            <div key={endpoint.key} className="rounded-card border border-hairline bg-ivory p-5">
              <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <code className="text-body-sm font-medium text-ink">{endpoint.path}</code>
                <span className="shrink-0 text-body-sm font-semibold tabular-nums text-amber">
                  {endpoint.priceNeb} {TOKEN_SYMBOL}
                </span>
              </div>
              <p className="mt-2 text-pretty text-body-sm text-slate">{endpoint.description}</p>
            </div>
          ))}
        </div>

        {/* DESIGN.md's Terminal Command Box — #060913 background, 1px
            border, 8px radius, monospace — the one other place in this
            file already uses this exact background (the Live stats
            section below) for the same "this is real infrastructure, not
            marketing copy" reason. */}
        <div className="mt-8 overflow-hidden rounded-lg border border-hairline bg-[#060913]">
          <div className="flex items-center gap-1.5 border-b border-white/10 px-4 py-2.5">
            <span className="h-2.5 w-2.5 rounded-full bg-white/15" aria-hidden="true" />
            <span className="h-2.5 w-2.5 rounded-full bg-white/15" aria-hidden="true" />
            <span className="h-2.5 w-2.5 rounded-full bg-white/15" aria-hidden="true" />
          </div>
          <pre className="overflow-x-auto p-4 text-caption leading-relaxed text-white/70">
            <code>{`$ curl -s ${SITE_NAME}/api/data/platform-stats
HTTP/1.1 402 Payment Required
PAYMENT-REQUIRED: eyJhY2NlcHRzIjpbeyJzY2hlbWUiOiJleGFjdCIs...
# decodes to: { scheme: "exact", network: "solana:...",
#               amount: "10000", asset: "<NEB mint>", payTo: "<treasury>" }

# sign a 0.01 NEB transfer to payTo, then retry with the payload
$ curl -s ${SITE_NAME}/api/data/platform-stats \\
    -H "PAYMENT-SIGNATURE: eyJwYXllciI6Ii4uLiIsInRyYW5zYWN0aW9uIjoiLi4uIn0="
HTTP/1.1 200 OK
PAYMENT-RESPONSE: eyJzZXR0bGVkIjp0cnVlLCJ0cmFuc2FjdGlvbiI6Ii4uLiJ9
{"ok":true,"data":{"totalApps":63,"totalTags":112,...}}`}</code>
          </pre>
        </div>
      </section>

      {/* Live stats — the same platform totals the Explore page shows,
          proof this is a real, running system rather than a mockup. */}
      {/* `reveal` lives on the inner wrapper, not this section: both it and
          `-translate-x-1/2` (the full-bleed breakout below) animate/set
          `transform`, and an animation's value wins outright for its whole
          duration — combining them here silently drops the horizontal
          centering the instant the reveal animation applies. */}
      <section className="relative left-1/2 w-screen -translate-x-1/2 overflow-hidden border-y border-hairline bg-[#040509] py-16">
        <div className="reveal mx-auto max-w-6xl px-4 sm:px-6 lg:px-8">
          <span className="text-body-sm font-semibold uppercase tracking-wide text-[#a5a8f5]">
            Right now
          </span>
          <h2 className="mt-2 text-balance font-display text-heading font-light text-white">
            Live on {SITE_NAME}
          </h2>
          <div className="mt-10 grid grid-cols-2 gap-6 sm:grid-cols-3 lg:grid-cols-5">
            <StatItem dark label="Apps" value={formatNumber(stats.totalApps)} />
            <StatItem dark label="Tags" value={formatNumber(stats.totalTags)} />
            <StatItem
              dark
              label="Votes cast"
              value={formatToken(stats.totalVoteWeight, TOKEN_SYMBOL)}
            />
            <StatItem dark label="Staked" value={formatToken(stats.totalStake, TOKEN_SYMBOL)} />
            <StatItem dark label="Page views" value={formatNumber(stats.totalViews)} />
          </div>
        </div>
      </section>

      {/* Explore CTA */}
      <section className="reveal mx-auto max-w-6xl px-4 sm:px-6 lg:px-8">
        <div className="grid items-center gap-10 rounded-card border border-hairline bg-ivory p-8 sm:grid-cols-2 sm:p-12">
          <div>
            <span className="text-body-sm font-semibold uppercase tracking-wide text-forest">
              Explore
            </span>
            <h2 className="mt-2 text-balance font-display text-heading font-light text-ink">
              See how it all connects
            </h2>
            <p className="mt-3 text-pretty text-body-sm text-slate">
              A live, force-directed map of every app and tag on the platform — clustered by
              shared tags, sized by stake. Drag a node, follow the connections, find something
              close to an app you already use.
            </p>
            <Link href="/explore" className="btn-primary mt-6 inline-flex">
              Open the map
            </Link>
          </div>
          <div
            className="relative hidden aspect-[4/3] overflow-hidden rounded-card border border-hairline sm:block"
            aria-hidden="true"
          >
            <ConstellationField className="absolute inset-0 h-full w-full" />
          </div>
        </div>
      </section>

      {/* Final CTA — same reveal/breakout split as the stats section above. */}
      <section className="relative left-1/2 w-screen -translate-x-1/2 overflow-hidden">
        <div
          className="absolute inset-0 -z-10"
          style={{ background: "radial-gradient(circle at 50% 0%, rgba(67, 56, 202, 0.08) 0%, rgba(255, 255, 255, 0) 60%)" }}
        />
        <div className="reveal mx-auto flex max-w-2xl flex-col items-center gap-5 px-4 py-24 text-center sm:px-6 lg:px-8">
          <h2 className="text-balance font-display text-heading font-light text-ink">
            Ready to back what&apos;s good?
          </h2>
          <p className="text-pretty text-body text-slate">
            Every vote and every tag stake earns {TOKEN_NAME} — on top of your principal, claimable
            anytime.
          </p>
          <Link href="/" className="btn-primary">
            Get started
          </Link>
        </div>
      </section>
    </div>
  );
}

function StatItem({
  label,
  value,
  dark,
}: {
  label: string;
  value: string;
  // This section (the "Live on nebulous.world" stats strip) is a deliberate
  // dark island on an otherwise light page — same bg-[#040509] treatment as
  // the terminal command box above it. `text-ink` resolves to near-black in
  // this theme, so it's invisible there; `dark` swaps in the same literal
  // white/white-70 escape hatch the terminal box uses instead of the
  // light-theme tokens.
  dark?: boolean;
}) {
  // Grid items get an implicit min-width: auto, sized to their content's
  // longest unbreakable run — at 30px bold this number can be wider than
  // its column, which pushed the whole grid (and, via the shared
  // containing block, every full-bleed section after it) past the
  // viewport. min-w-0 lets the column actually shrink to its track width;
  // splitting the unit onto its own line (like MetricTrendCard) gives long
  // values ("133.96K NEB") a natural wrap point instead of overflowing.
  const [amount, unit] = splitValueUnit(value);
  return (
    <div className="min-w-0">
      <div className="flex flex-wrap items-baseline gap-x-1.5">
        <span
          className={cn(
            "font-display text-heading-sm font-normal tabular-nums",
            dark ? "text-white" : "text-ink",
          )}
        >
          {amount}
        </span>
        {unit && (
          <span className={cn("text-body-sm", dark ? "text-white/70" : "text-slate-steel")}>
            {unit}
          </span>
        )}
      </div>
      <div
        className={cn(
          "mt-1 text-caption uppercase tracking-wide",
          dark ? "text-white/70" : "text-slate-steel",
        )}
      >
        {label}
      </div>
    </div>
  );
}
