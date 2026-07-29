/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Old routes folded into other pages — redirect rather than 404.
  async redirects() {
    return [
      // Buying NEB moved into /rewards, alongside pool analytics and reward claiming.
      { source: "/token", destination: "/rewards", permanent: true },
      // App submission moved into a "Create app" modal on the Discover page.
      { source: "/submit", destination: "/", permanent: true },
    ];
  },
  images: {
    remotePatterns: [
      { protocol: "https", hostname: "**" },
      { protocol: "http", hostname: "localhost" },
    ],
    // The three settings below exist to keep the image optimizer inside the
    // web service's 512MB instance (see render.yaml). Every `src` here is a
    // third-party URL off an App row — an OpenGraph image or a favicon on
    // someone else's host — so the optimizer is decoding arbitrary internet
    // images in-process, and sharp holds the whole decoded bitmap while it
    // works. Measured: six concurrent optimizations of one 4000x2100 JPEG
    // OOM-kill a 512MB container from a 105MB baseline.
    //
    // Widths are an allowlist: Next 400s any `w` outside deviceSizes +
    // imageSizes before allocating anything, so trimming this list is a real
    // guard and not just a hint. 1200 is the ceiling because the sources are
    // 1200x630 OpenGraph assets — the 1920/2048/3840 entries in Next's
    // default only ever bought us an upscale of a 1200px original, at ~10x
    // the peak memory of a 1200px resize. Googlebot-Image was requesting
    // w=3840 in production.
    deviceSizes: [640, 828, 1080, 1200],
    // Small fixed-size uses: the 20px stake rows (MyStakes), the 32px navbar
    // logo, and the 340px ad slot — each needs its 1x and 2x bucket.
    imageSizes: [32, 64, 128, 384],
    // Next's default is 60 seconds, which meant the same unchanged app icon
    // was re-fetched and re-encoded from scratch roughly every minute a
    // crawler came back — production logs show one image optimized ~18x in a
    // day, 2ms cache hits interleaved with 1-2s misses. App icons change
    // about never, and a changed icon lands on a new URL (so a new cache
    // key), which makes a long TTL safe: the work is paid once per image
    // instead of continuously.
    minimumCacheTTL: 60 * 60 * 24 * 31,
  },
  webpack: (config) => {
    // Solana / wallet-adapter pull in optional native deps we don't need in the browser.
    config.externals = config.externals || [];
    config.externals.push("pino-pretty", "lokijs", "encoding");
    // @solana/wallet-adapter-walletconnect -> @reown/appkit's full network
    // list -> viem's chain configs pull in ox's tempo module, which resolves
    // a dependency via a runtime expression rather than a static string —
    // legitimate library code (dynamic i18n-style locale/config loading),
    // not a bug, but webpack can't statically analyze it and spams a
    // "Critical dependency" warning per module that imports it, every
    // rebuild. This app has no EVM chain in its wallet config (Solana
    // only), so there's nothing real to miss by not treating an
    // unresolvable expression-based require as build-breaking here.
    config.module.exprContextCritical = false;
    return config;
  },
};

export default nextConfig;
