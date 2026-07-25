import { notFound } from "next/navigation";
import Link from "next/link";
import type { Metadata } from "next";
import { fetchTagBySlug, searchApps } from "@/lib/indexerClient";
import { formatToken, formatNumber } from "@/lib/utils";
import { TOKEN_SYMBOL, SITE_URL, SITE_NAME } from "@/lib/constants";
import { AppCard } from "@/components/AppCard";
import { JsonLd } from "@/components/JsonLd";
import { AutoRefresh } from "@/components/AutoRefresh";

export const dynamic = "force-dynamic";

interface Props {
  params: Promise<{ slug: string }>;
}

// A long-tail SEO landing page per tag — "solana defi apps", "ai chatbot
// tools", etc. are exactly the kind of query these can independently rank
// for, the way a marketplace's category pages do, which a single faceted
// "/?tags=" search parameter never can (see page.tsx's own noindex on
// filtered search results — this is the crawlable, canonical counterpart).
const APPS_LIMIT = 24;

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const { slug } = await params;
  const tag = await fetchTagBySlug(slug);
  if (!tag) return { title: "Tag not found" };

  const title = `#${tag.name} apps`;
  const description = `${formatNumber(tag.appCount)} app${tag.appCount === 1 ? "" : "s"} tagged #${tag.name} on ${SITE_NAME}, ranked by token-weighted votes and tag stake.`;
  const url = `${SITE_URL}/tags/${tag.slug}`;

  return {
    title,
    description,
    alternates: { canonical: url },
    openGraph: { title, description, url, siteName: SITE_NAME, locale: "en_US", type: "website" },
    twitter: { card: "summary_large_image", title, description },
  };
}

export default async function TagPage({ params }: Props) {
  const { slug } = await params;
  const tag = await fetchTagBySlug(slug);
  if (!tag) notFound();

  const { apps, total } = await searchApps({
    q: "",
    tags: [tag.slug],
    fuzzy: "",
    sort: "rank",
    page: 1,
    pageSize: APPS_LIMIT,
  });

  const url = `${SITE_URL}/tags/${tag.slug}`;
  const breadcrumbLd = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      { "@type": "ListItem", position: 1, name: SITE_NAME, item: SITE_URL },
      { "@type": "ListItem", position: 2, name: `#${tag.name}`, item: url },
    ],
  };
  const listLd = {
    "@context": "https://schema.org",
    "@type": "CollectionPage",
    name: `#${tag.name} apps`,
    url,
    mainEntity: {
      "@type": "ItemList",
      numberOfItems: total,
      itemListElement: apps.map((app, i) => ({
        "@type": "ListItem",
        position: i + 1,
        url: `${SITE_URL}/app/${app.slug}`,
        name: app.name,
      })),
    },
  };

  return (
    <div className="space-y-6">
      <AutoRefresh />
      <JsonLd data={breadcrumbLd} />
      <JsonLd data={listLd} />

      <Link href="/" className="text-sm text-slate hover:text-ink">
        ← Back to discover
      </Link>

      <div className="card p-6">
        <h1 className="font-display text-heading-sm font-bold text-ink">
          #{tag.name}
        </h1>
        <div className="mt-3 flex gap-6 border-t border-hairline pt-4">
          <div>
            <div className="text-caption uppercase tracking-wide text-slate">Apps</div>
            <div className="mt-1 text-xl font-semibold text-ink">{formatNumber(tag.appCount)}</div>
          </div>
          <div>
            <div className="text-caption uppercase tracking-wide text-slate">Total staked</div>
            <div className="mt-1 text-xl font-semibold text-ink">
              {formatToken(tag.stakeTotal, TOKEN_SYMBOL)}
            </div>
          </div>
        </div>
      </div>

      {apps.length === 0 ? (
        <div className="card grid place-items-center p-12 text-center text-slate-steel">
          No apps tagged #{tag.name} yet.
        </div>
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {apps.map((app, i) => (
              <AppCard key={app.id} app={app} rank={i + 1} />
            ))}
          </div>
          {total > apps.length && (
            <div className="text-center">
              <Link
                href={`/?tags=${encodeURIComponent(tag.slug)}`}
                className="text-sm font-medium text-cobalt hover:underline"
              >
                See all {formatNumber(total)} apps tagged #{tag.name} →
              </Link>
            </div>
          )}
        </>
      )}
    </div>
  );
}
