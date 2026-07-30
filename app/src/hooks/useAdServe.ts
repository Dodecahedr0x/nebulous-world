"use client";

import { useEffect, useRef, useState } from "react";

export interface ServedAd {
  id: string;
  title: string;
  body: string;
  imageUrl: string | null;
  targetUrl: string;
}

/**
 * Requests a sponsored ad for `appId` (recording a revenue-bearing
 * impression) and reports clicks. Re-requests whenever `appId` changes —
 * for AdSlot the appId is static for the page's lifetime, so this degrades
 * to a one-shot request there.
 */
export function useAdServe(appId: string) {
  const [ad, setAd] = useState<ServedAd | null>(null);
  const [impressionId, setImpressionId] = useState<string | null>(null);
  const requestedFor = useRef<string | null>(null);

  useEffect(() => {
    if (requestedFor.current === appId) return;
    requestedFor.current = appId;
    setAd(null);
    setImpressionId(null);
    fetch("/api/ads/serve", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ appId, path: window.location.pathname }),
    })
      .then((r) => r.json())
      .then((json) => {
        if (json.ok && json.data.ad) {
          setAd(json.data.ad);
          setImpressionId(json.data.impressionId ?? null);
        }
      })
      .catch(() => {});
  }, [appId]);

  const onClick = () => {
    if (impressionId) {
      fetch("/api/ads/click", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ impressionId }),
        keepalive: true,
      }).catch(() => {});
    }
  };

  return { ad, onClick };
}
