import type { Metadata } from "next";
import { PageHeader } from "@/components/PageHeader";
import { FindFunnel } from "@/components/find/FindFunnel";
import { FUNNEL_ANSWERS_PARAM } from "@/components/find/funnelState";
import { fetchNextFindQuestion } from "@/lib/indexerClient";
import { FIND_PAGE_TITLE, FIND_PAGE_DESCRIPTION, SITE_URL } from "@/lib/constants";

export const dynamic = "force-dynamic";

interface PageProps {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}

// The funnel mirrors its answer history into the query string (A82), so any
// query string on /find is a resumed or shared in-progress link rather than a
// distinct page worth indexing. Load-bearing rather than theoretical now that
// every answered question is its own URL. Same shape as the homepage's
// isFilteredSearch, kept local because the two pages disagree on what counts
// as a parameter worth ignoring.
function isParameterized(sp: Record<string, string | string[] | undefined>): boolean {
  return Object.entries(sp).some(([, value]) => {
    if (value === undefined || value === "") return false;
    if (Array.isArray(value) && value.length === 0) return false;
    return true;
  });
}

export async function generateMetadata({ searchParams }: PageProps): Promise<Metadata> {
  const sp = await searchParams;
  return {
    title: FIND_PAGE_TITLE,
    description: FIND_PAGE_DESCRIPTION,
    alternates: { canonical: `${SITE_URL}/find` },
    // `follow`, so crawlers still reach every /app/[slug] a shared funnel
    // state links to — only the near-duplicate state itself is dropped.
    ...(isParameterized(sp) ? { robots: { index: false, follow: true } } : {}),
  };
}

export default async function FindPage({ searchParams }: PageProps) {
  // Every answered question pushes a new URL, and this route is force-dynamic,
  // so each one re-renders the page on the server. Asking the indexer for
  // question 1 again there would be a wasted round trip per answer: a resuming
  // URL already describes a funnel past that point, and the client fetches the
  // question for its own history.
  const resuming = (await searchParams)[FUNNEL_ANSWERS_PARAM] !== undefined;
  // Same degrade-gracefully reasoning as rankings/page.tsx: the engine lives
  // in the indexer, a separate service that can be unreachable. A null result
  // is not an error state — FindFunnel just fetches the first question on
  // mount instead of rendering it server-side.
  const initialResult = resuming
    ? null
    : await fetchNextFindQuestion({ answers: [] }).catch(() => null);

  return (
    <div className="space-y-6">
      <PageHeader title={FIND_PAGE_TITLE} description={FIND_PAGE_DESCRIPTION} />
      <div className="max-w-3xl">
        <FindFunnel initialResult={initialResult} />
      </div>
    </div>
  );
}
