"use client";

import Script from "next/script";

/**
 * Loads Google's global AdSense site tag (Auto ads) — the actual revenue
 * source `scripts/settleEpoch.ts`/`src/lib/adsense.ts` pull real earnings
 * from. Distinct from, and unrelated to, `components/ads/AdSlot.tsx`/
 * `AdCard.tsx`: those render this platform's own internal "sponsored app"
 * marketplace (self-serve, simulated CPM via `AD_CPM`) — not a Google ad
 * unit — and their revenue never touches AdSense. Before this component
 * existed, nothing in the app ever loaded adsbygoogle.js, so no real
 * AdSense ad could ever render regardless of how the account/settlement
 * pipeline was configured.
 *
 * Mounted once, in the root layout, so it's present on every page — Auto
 * ads decides placement itself, no explicit `<ins>` ad unit needed here.
 * No-ops when unconfigured (e.g. local dev), same pattern as
 * `TrafficBeacon`'s Turnstile loading.
 */
export function AdSenseScript() {
  const publisherId = process.env.NEXT_PUBLIC_ADSENSE_PUBLISHER_ID;
  if (!publisherId) return null;

  return (
    <Script
      async
      src={`https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js?client=${publisherId}`}
      crossOrigin="anonymous"
      strategy="afterInteractive"
    />
  );
}
