import { notFound } from "next/navigation";
import Image from "next/image";
import Link from "next/link";
import type { Metadata } from "next";
import { fetchAppBySlug } from "@/lib/indexerClient";
import { formatToken, shortAddress, timeAgo, hostname, topStakedTag } from "@/lib/utils";
import { TOKEN_SYMBOL, SITE_URL, SITE_NAME } from "@/lib/constants";
import { VotePanel } from "@/components/app/VotePanel";
import { TagStakePanel } from "@/components/app/TagStakePanel";
import { TrafficBeacon } from "@/components/app/TrafficBeacon";
import { AdSlot } from "@/components/ads/AdSlot";
import { AppMetricsPanel } from "@/components/app/AppMetricsPanel";
import { JsonLd } from "@/components/JsonLd";
import { AutoRefresh } from "@/components/AutoRefresh";

export const dynamic = "force-dynamic";

interface Props {
  params: Promise<{ slug: string }>;
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { slug } = await params;
  const detail = await fetchAppBySlug(slug);
  if (!detail) return { title: "App not found" };

  const { app } = detail;
  const description = app.tagline || app.description;
  const url = `${SITE_URL}/app/${app.slug}`;

  return {
    title: app.name,
    description,
    alternates: { canonical: url },
    openGraph: {
      title: app.name,
      description,
      url,
      siteName: SITE_NAME,
      locale: "en_US",
      type: "website",
    },
    twitter: {
      card: "summary_large_image",
      title: app.name,
      description,
    },
  };
}

export default async function AppDetailPage({ params }: Props) {
  const { slug } = await params;
  const detail = await fetchAppBySlug(slug);
  if (!detail) notFound();

  const { app, recentVotes, topStakers, snapshots } = detail;
  const topTag = topStakedTag(app.tags);

  // Structured data for search engines — the crowd-sourced stats behind the
  // OpenGraph card (vote/view counts) expressed via schema.org's
  // InteractionCounter rather than aggregateRating: these are engagement
  // tallies, not a 1-5 star review system, and misrepresenting one as the
  // other risks a Google structured-data manual action.
  const appLd = {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: app.name,
    description: app.tagline || app.description,
    url: `${SITE_URL}/app/${app.slug}`,
    ...(app.iconUrl ? { image: app.iconUrl } : {}),
    applicationCategory: app.category,
    operatingSystem: "Web",
    interactionStatistic: [
      {
        "@type": "InteractionCounter",
        interactionType: "https://schema.org/LikeAction",
        userInteractionCount: app.voteCount,
      },
      {
        "@type": "InteractionCounter",
        interactionType: "https://schema.org/ViewAction",
        userInteractionCount: app.viewCount,
      },
    ],
  };

  // Lets Google render "nebulous.world > App Name" as the SERP breadcrumb
  // instead of the raw URL — small CTR bump, no visual change on the page
  // itself (there's no on-page breadcrumb UI to keep in sync).
  const breadcrumbLd = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      { "@type": "ListItem", position: 1, name: SITE_NAME, item: SITE_URL },
      { "@type": "ListItem", position: 2, name: app.name, item: `${SITE_URL}/app/${app.slug}` },
    ],
  };

  return (
    <div className="space-y-6">
      <AutoRefresh />
      <JsonLd data={appLd} />
      <JsonLd data={breadcrumbLd} />
      {/* Records a page view for traffic analytics & revenue attribution. */}
      <TrafficBeacon appId={app.id} path={`/app/${app.slug}`} />

      <Link href="/" className="text-sm text-slate hover:text-ink">
        ← Back to discover
      </Link>

      {/* Header — an OpenGraph link-preview card (hero image, domain strip,
          title, description, top tag/chain), the same shape a shared link
          unfurls into. This is also the app's "About": its description
          lives here instead of a separate panel. */}
      <div className="card overflow-hidden p-0">
        <div className="relative aspect-[3/1] w-full shrink-0 overflow-hidden bg-mist">
          {app.iconUrl ? (
            <Image
              src={app.iconUrl}
              alt=""
              fill
              sizes="(min-width: 1024px) 800px, 100vw"
              priority
              className="object-cover ring-1 ring-inset ring-black/10"
            />
          ) : (
            <div className="grid h-full w-full place-items-center text-5xl font-bold text-violet">
              {app.name.charAt(0).toUpperCase() || "?"}
            </div>
          )}
          <div className="absolute right-3 top-3 flex gap-2">
            {topTag && (
              <Link
                href={`/tags/${topTag.slug}`}
                className="chip border-none bg-white/90 shadow-subtle hover:bg-white"
              >
                #{topTag.name}
              </Link>
            )}
            <span className="chip border-none bg-white/90 capitalize shadow-subtle">
              {app.chain}
            </span>
          </div>
        </div>
        <div className="flex flex-col gap-4 border-t border-hairline bg-ivory/60 p-6 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <span className="text-[11px] font-medium uppercase tracking-wide text-slate-steel">
              {hostname(app.url)}
            </span>
            <h1 className="font-display text-balance text-2xl font-bold text-ink">
              {app.name}
            </h1>
            <p className="mt-1 whitespace-pre-line text-pretty text-slate">
              {app.description || app.tagline}
            </p>
          </div>
          <a
            href={app.url}
            target="_blank"
            rel="noopener noreferrer nofollow"
            className="btn-primary shrink-0"
          >
            Visit app ↗
          </a>
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-[1fr_340px]">
        <div className="space-y-6">
          <AppMetricsPanel
            snapshots={snapshots}
            current={{
              rankScore: app.rankScore,
              voteWeight: app.voteWeight,
              stakeTotal: app.stakeTotal,
              viewCount: app.viewCount,
            }}
          />

          {/* Recent activity */}
          <section className="card p-6">
            <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate">
              Recent votes
            </h2>
            {recentVotes.length === 0 ? (
              <p className="text-sm text-slate-steel">
                No votes yet — be the first to vote.
              </p>
            ) : (
              <ul className="divide-y divide-hairline">
                {recentVotes.map((v) => (
                  <li
                    key={v.id}
                    className="flex items-center justify-between py-2 text-sm"
                  >
                    <span className="font-mono text-slate">
                      {v.wallet.length > 20 ? shortAddress(v.wallet) : v.wallet}
                    </span>
                    <span className="text-cobalt">
                      +{formatToken(v.amount, TOKEN_SYMBOL)}
                    </span>
                    <span className="text-xs text-slate-steel">
                      {timeAgo(v.createdAt)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>

        {/* Sidebar — every way to back this app or its tags lives here. */}
        <div className="space-y-6">
          <VotePanel appId={app.id} />

          <TagStakePanel appId={app.id} tags={app.tags} />

          <AdSlot appId={app.id} />

          <section className="card p-6">
            <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate">
              Top stakers
            </h2>
            {topStakers.length === 0 ? (
              <p className="text-sm text-slate-steel">No stakers yet.</p>
            ) : (
              <ul className="space-y-2">
                {topStakers.map((s, i) => (
                  <li
                    key={i}
                    className="flex items-center justify-between text-sm"
                  >
                    <span className="font-mono text-slate">
                      {s.wallet.length > 20 ? shortAddress(s.wallet) : s.wallet}
                    </span>
                    <span className="text-ink">
                      {formatToken(s.amount, TOKEN_SYMBOL)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
