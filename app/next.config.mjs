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
