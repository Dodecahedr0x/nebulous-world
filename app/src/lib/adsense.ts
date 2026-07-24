// Thin wrapper around the AdSense Management API v2's reports:generate
// endpoint, plus the OAuth2 token refresh that feeds it an access token.
//
// AdSense's Management API has no service-account auth path — only user
// OAuth consent (see https://developers.google.com/adsense/management/oauth).
// A short-lived access token minted by hand would expire (~1hr) long before
// the next scheduled settlement run, so `getAdsenseAccessToken` instead
// mints a fresh one on every call from a long-lived refresh token — the
// standard pattern for unattended server-side access to a Google API that
// doesn't support service accounts. The refresh token itself is obtained
// once, out-of-band, via a manual OAuth consent flow (e.g. Google's OAuth
// Playground at https://developers.google.com/oauthplayground, using this
// app's own client id/secret and the `adsense.readonly` scope) and doesn't
// expire on its own — only on manual revocation or ~6 months of disuse.

export interface EarningsPeriod {
  start: Date;
  end: Date;
}

function isoDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

/**
 * Exchanges the long-lived `ADSENSE_REFRESH_TOKEN` for a fresh, short-lived
 * access token. Call this immediately before `fetchAdsenseEarnings` — the
 * token isn't cached across calls, since settlement only runs a handful of
 * times a week and a fresh token is one extra request, not worth the
 * complexity of tracking expiry.
 */
export async function getAdsenseAccessToken(): Promise<string> {
  const clientId = process.env.ADSENSE_CLIENT_ID;
  const clientSecret = process.env.ADSENSE_CLIENT_SECRET;
  const refreshToken = process.env.ADSENSE_REFRESH_TOKEN;
  if (!clientId || !clientSecret || !refreshToken) {
    throw new Error(
      "ADSENSE_CLIENT_ID / ADSENSE_CLIENT_SECRET / ADSENSE_REFRESH_TOKEN must all be configured",
    );
  }

  const res = await fetch("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      client_id: clientId,
      client_secret: clientSecret,
      refresh_token: refreshToken,
      grant_type: "refresh_token",
    }),
  });
  if (!res.ok) {
    throw new Error(`AdSense token refresh failed ${res.status}: ${await res.text()}`);
  }
  const json = (await res.json()) as { access_token?: string };
  if (!json.access_token) {
    throw new Error("AdSense token refresh response had no access_token");
  }
  return json.access_token;
}

export async function fetchAdsenseEarnings(
  period: EarningsPeriod,
  accessToken: string,
): Promise<number> {
  const accountId = process.env.ADSENSE_ACCOUNT_ID;
  if (!accountId) throw new Error("ADSENSE_ACCOUNT_ID is not configured");

  const url = new URL(
    `https://adsense.googleapis.com/v2/accounts/${accountId}/reports:generate`,
  );
  url.searchParams.set("dateRange", "CUSTOM");
  url.searchParams.set("startDate.year", String(period.start.getUTCFullYear()));
  url.searchParams.set("startDate.month", String(period.start.getUTCMonth() + 1));
  url.searchParams.set("startDate.day", String(period.start.getUTCDate()));
  url.searchParams.set("endDate.year", String(period.end.getUTCFullYear()));
  url.searchParams.set("endDate.month", String(period.end.getUTCMonth() + 1));
  url.searchParams.set("endDate.day", String(period.end.getUTCDate()));
  url.searchParams.set("metrics", "ESTIMATED_EARNINGS");

  console.log(`[adsense] fetching earnings for ${isoDate(period.start)}..${isoDate(period.end)}`);

  const res = await fetch(url, { headers: { Authorization: `Bearer ${accessToken}` } });
  if (!res.ok) {
    throw new Error(`AdSense API error ${res.status}: ${await res.text()}`);
  }
  const json = (await res.json()) as { totals?: { cells?: { value?: string }[] } };
  const raw = json.totals?.cells?.[json.totals.cells.length - 1]?.value ?? "0";
  return parseFloat(raw);
}
