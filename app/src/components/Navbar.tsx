"use client";

import { useEffect, useState } from "react";
import Image from "next/image";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { useWallet } from "@solana/wallet-adapter-react";
import { ConnectButton } from "@/components/ConnectButton";
import { useUserLevel } from "@/hooks/useUserLevel";
import { useWalletBalances } from "@/hooks/useWalletBalances";
import { config } from "@/lib/config";
import { TOKEN_SYMBOL } from "@/lib/constants";
import { cn, formatToken } from "@/lib/utils";

const NAV = [
  { href: "/", label: "Browse" },
  { href: "/rankings", label: "Rankings" },
  { href: "/rewards", label: "Rewards" },
  { href: "/about", label: "About" },
];

export function Navbar() {
  const pathname = usePathname();
  const { connected } = useWallet();
  const { neb } = useWalletBalances(config.solana.voteTokenMint || null, null);
  const userLevel = useUserLevel();
  const [mobileOpen, setMobileOpen] = useState(false);

  // Below md, the 4-link nav lives in a dropdown instead — close it on any
  // route change (a link click, back/forward, or a deep link elsewhere in
  // the app) so it never lingers open over the new page.
  useEffect(() => {
    setMobileOpen(false);
  }, [pathname]);

  return (
    <header className="sticky top-0 z-40 border-b border-hairline bg-cream">
      <div className="mx-auto flex h-16 w-full max-w-7xl items-center justify-between gap-4 px-4 sm:px-6 lg:px-8">
        <div className="flex items-center gap-8">
          <Link href="/" className="flex items-center gap-2">
            <Image
              src="/nebulous_logo.png"
              alt=""
              width={32}
              height={32}
              priority
              className="h-8 w-8 rounded-icon"
            />
            <span className="text-base font-bold tracking-tight text-ink sm:text-lg">
              nebulous.<span className="text-cobalt">world</span>
            </span>
          </Link>
          <nav className="hidden items-center gap-1 md:flex">
            {NAV.map((item) => {
              const active =
                item.href === "/"
                  ? pathname === "/"
                  : pathname.startsWith(item.href);
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  className={cn(
                    "rounded-navitem px-3 py-2 text-sm font-medium transition-colors duration-150",
                    active
                      ? "bg-indigo-soft text-cobalt"
                      : "text-slate hover:text-ink",
                  )}
                >
                  {item.label}
                </Link>
              );
            })}
          </nav>
        </div>
        <div className="flex items-center gap-1 sm:gap-2">
          {connected && (
            <span
              className="h-1.5 w-1.5 rounded-full bg-forest"
              aria-hidden="true"
            />
          )}
          {connected && userLevel && (
            <Link href="/profile" className="chip chip-active font-mono tabular-nums">
              Lv {userLevel.level}
            </Link>
          )}
          {/* Hidden below sm: the widest, least-essential piece of header
              content — on a narrow phone with a wallet connected, the
              logo/wordmark + level chip + this + Connect button together
              would push the sticky header into horizontal overflow. */}
          {connected && neb !== null && (
            <span className="chip hidden font-mono tabular-nums sm:inline-flex">
              {formatToken(neb, TOKEN_SYMBOL)}
            </span>
          )}
          <ConnectButton />
          <button
            type="button"
            onClick={() => setMobileOpen((v) => !v)}
            className="rounded-navitem p-1.5 text-slate hover:text-ink md:hidden"
            aria-expanded={mobileOpen}
            aria-controls="mobile-nav"
            aria-label="Toggle navigation menu"
          >
            <svg className="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              {mobileOpen ? (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              ) : (
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h16M4 18h16" />
              )}
            </svg>
          </button>
        </div>
      </div>

      {mobileOpen && (
        <nav
          id="mobile-nav"
          aria-label="Navigation"
          className="flex flex-col gap-1 border-t border-hairline bg-cream px-4 py-2 md:hidden"
        >
          {NAV.map((item) => {
            const active =
              item.href === "/" ? pathname === "/" : pathname.startsWith(item.href);
            return (
              <Link
                key={item.href}
                href={item.href}
                className={cn(
                  "rounded-navitem px-3 py-2 text-sm font-medium transition-colors duration-150",
                  active ? "bg-indigo-soft text-cobalt" : "text-slate hover:text-ink",
                )}
              >
                {item.label}
              </Link>
            );
          })}
        </nav>
      )}
    </header>
  );
}
