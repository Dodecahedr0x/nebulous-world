import type { Metadata, Viewport } from "next";
import { Inter } from "next/font/google";
import "./globals.css";
import { SolanaProvider } from "@/components/providers/SolanaProvider";
import { AuthProvider } from "@/components/providers/AuthProvider";
import { AppShell } from "@/components/AppShell";
import { Toaster } from "@/components/ui/Toaster";
import { JsonLd } from "@/components/JsonLd";
import { AdSenseScript } from "@/components/ads/AdSenseScript";
import { SITE_NAME, SITE_DESCRIPTION, SITE_URL } from "@/lib/constants";

const title = `${SITE_NAME} — discover the best apps, ranked by the crowd`;

// One typeface for the whole app now — see DESIGN.md. `font-display` in
// tailwind.config.ts resolves to this same `--font-ui-sans-serif` variable
// (no separate `--font-obviously` variable exists any more), so nothing
// else needs to change for lingering `font-display` class usage to keep
// working correctly.
const bodySans = Inter({
  subsets: ["latin"],
  variable: "--font-ui-sans-serif",
  weight: ["300", "400", "500", "600", "700"],
});

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: { default: title, template: `%s — ${SITE_NAME}` },
  description: SITE_DESCRIPTION,
  icons: { icon: "/favicon.ico" },
  openGraph: {
    title,
    description: SITE_DESCRIPTION,
    url: SITE_URL,
    siteName: SITE_NAME,
    type: "website",
    locale: "en_US",
  },
  twitter: {
    card: "summary_large_image",
    title,
    description: SITE_DESCRIPTION,
  },
};

export const viewport: Viewport = {
  themeColor: "#ffffff",
};

// Site-level structured data — lets Google offer a sitelinks search box for
// the site straight from search results, pointing at the home page's own
// `?q=` search (see components/discover/Discover.tsx).
const siteLd = {
  "@context": "https://schema.org",
  "@type": "WebSite",
  name: SITE_NAME,
  url: SITE_URL,
  potentialAction: {
    "@type": "SearchAction",
    target: `${SITE_URL}/?q={search_term_string}`,
    "query-input": "required name=search_term_string",
  },
};

// Brand entity — feeds Google's knowledge panel eligibility. `logo` needs
// to be at least 112x112 per Google's guidance; apple-icon.png (180x180,
// see app/apple-icon.png) clears that, icon.png (32x32) wouldn't.
const orgLd = {
  "@context": "https://schema.org",
  "@type": "Organization",
  name: SITE_NAME,
  url: SITE_URL,
  logo: `${SITE_URL}/apple-icon.png`,
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className={bodySans.variable}>
      <body className="bg-cream font-sans text-ink antialiased">
        <AdSenseScript />
        <JsonLd data={siteLd} />
        <JsonLd data={orgLd} />
        <SolanaProvider>
          <AuthProvider>
            <Toaster>
              <AppShell>{children}</AppShell>
            </Toaster>
          </AuthProvider>
        </SolanaProvider>
      </body>
    </html>
  );
}
