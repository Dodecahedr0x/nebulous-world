"use client";

import { useEffect, useRef, useState } from "react";
import Script from "next/script";

declare global {
  interface Window {
    turnstile?: {
      render: (
        el: HTMLElement,
        opts: {
          sitekey: string;
          callback: (token: string) => void;
          size: "invisible";
        },
      ) => void;
    };
  }
}

// If Turnstile's own challenge never resolves (a site key not registered for
// this domain, an ad-blocker or privacy extension blocking
// challenges.cloudflare.com, a network hiccup reaching Cloudflare), its
// `callback` simply never fires, and with nothing else driving `send()` this
// view would go permanently, silently untracked — no console error, nothing
// to notice. This bounds how long a view can wait on Turnstile before
// falling back to counting it (as not revenue-eligible) anyway, the same
// place `viewCount` reads from either way.
const TURNSTILE_TIMEOUT_MS = 4000;

/**
 * Fires a single tracking beacon when an app page mounts. Server-side dedupe
 * (per visitor/session) means refreshes and re-mounts won't double-count.
 *
 * Also renders an invisible Cloudflare Turnstile challenge and sends its
 * token along with the beacon — the server only marks a view
 * revenue-eligible once that token verifies (see src/lib/turnstile.ts). If no
 * site key is configured (e.g. local dev), the beacon fires without a token
 * and the view is simply tracked as non-revenue-eligible — same fallback
 * TURNSTILE_TIMEOUT_MS uses when a site key IS configured but the challenge
 * doesn't resolve in time.
 */
export function TrafficBeacon({
  appId,
  path,
}: {
  appId: string;
  path: string;
}) {
  const sent = useRef(false);
  const widgetRef = useRef<HTMLDivElement>(null);
  // Set once next/script's onReady fires — `window.turnstile` isn't
  // necessarily defined yet at this component's own mount time, since the
  // Cloudflare script (loaded via `strategy="afterInteractive"`) finishes
  // loading asynchronously, on its own schedule, independent of this
  // component's effects. Checking `window.turnstile` synchronously on mount
  // (the old approach) meant the widget almost never actually rendered in
  // practice — the check nearly always ran before the script had loaded —
  // silently falling back to the no-token path every time, site key or not.
  const [turnstileReady, setTurnstileReady] = useState(false);

  useEffect(() => {
    sent.current = false;

    function send(turnstileToken: string | null) {
      if (sent.current) return;
      sent.current = true;
      const referrer =
        typeof document !== "undefined" ? document.referrer : undefined;
      fetch("/api/track", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ appId, path, referrer, turnstileToken }),
        keepalive: true,
      }).catch(() => {
        /* tracking is best-effort */
      });
    }

    const siteKey = process.env.NEXT_PUBLIC_TURNSTILE_SITE_KEY;
    if (!siteKey || !widgetRef.current) {
      // No CAPTCHA configured (e.g. local dev) — track without a token; the
      // server marks such views as not revenue-eligible.
      send(null);
      return;
    }

    const timeoutId = window.setTimeout(() => send(null), TURNSTILE_TIMEOUT_MS);

    if (turnstileReady && window.turnstile) {
      window.turnstile.render(widgetRef.current, {
        sitekey: siteKey,
        size: "invisible",
        callback: send,
      });
    }

    return () => window.clearTimeout(timeoutId);
  }, [appId, path, turnstileReady]);

  return (
    <>
      <Script
        src="https://challenges.cloudflare.com/turnstile/v0/api.js"
        strategy="afterInteractive"
        onReady={() => setTurnstileReady(true)}
      />
      <div ref={widgetRef} />
    </>
  );
}
