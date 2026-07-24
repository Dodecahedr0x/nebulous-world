"use client";

import { useWallet } from "@solana/wallet-adapter-react";
import { useWalletModal } from "@solana/wallet-adapter-react-ui";
import { useAuth } from "@/components/providers/AuthProvider";
import { useToast } from "@/components/ui/Toaster";
import { shortAddress } from "@/lib/utils";

/**
 * Walks the user through connect → sign-in (see AuthProvider's SIWS flow)
 * and, once authenticated, shows their wallet identity — clicking that
 * disconnects. Three states: not connected ("Connect wallet"), connected
 * but not yet signed in ("Sign in" — a message-signing prompt, no
 * transaction/fee), and signed in (the identity chip).
 */
export function ConnectButton() {
  const { connected, connecting } = useWallet();
  const { setVisible } = useWalletModal();
  const { user, signIn, signOut, signingIn } = useAuth();
  const toast = useToast();

  if (!connected) {
    return (
      <button
        className="btn-primary shrink-0 whitespace-nowrap px-3 py-2 md:px-6 md:py-3"
        disabled={connecting}
        onClick={() => setVisible(true)}
      >
        {connecting ? (
          "Connecting…"
        ) : (
          <>
            <span className="md:hidden">Connect</span>
            <span className="hidden md:inline">Connect wallet</span>
          </>
        )}
      </button>
    );
  }

  if (!user) {
    return (
      <button
        className="btn-primary shrink-0 whitespace-nowrap px-3 py-2 md:px-6 md:py-3"
        disabled={signingIn}
        onClick={async () => {
          const signedIn = await signIn();
          if (signedIn) toast.success("Signed in");
        }}
      >
        {signingIn ? "Check wallet…" : "Sign in"}
      </button>
    );
  }

  return (
    <button
      type="button"
      className="chip chip-active shrink-0 whitespace-nowrap font-mono transition-colors duration-150 hover:border-negative/60 hover:text-negative"
      onClick={() => signOut()}
      title="Click to disconnect"
    >
      {user.handle ?? shortAddress(user.wallet)}
    </button>
  );
}
