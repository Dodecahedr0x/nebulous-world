"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import bs58 from "bs58";

export interface AuthUser {
  id: string;
  wallet: string;
  handle: string | null;
}

interface AuthState {
  user: AuthUser | null;
  loading: boolean;
  signingIn: boolean;
  error: string | null;
  signIn: () => Promise<boolean>;
  signOut: () => Promise<void>;
  refresh: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);

/**
 * Handles Sign-In-With-Solana: fetch a nonce, ask the connected wallet to sign
 * the challenge message, and exchange the signature for a session cookie.
 * This is deliberate friction over auto-connecting a session the moment a
 * wallet connects — the signature is what lets the session (and the rate
 * limiter in lib/api.ts's requireRateLimit) actually identify a unique
 * wallet, rather than trusting whatever address a client claims for free.
 */
export function AuthProvider({ children }: { children: ReactNode }) {
  const { publicKey, signMessage, connected, disconnect } = useWallet();
  const [user, setUser] = useState<AuthUser | null>(null);
  const [loading, setLoading] = useState(true);
  const [signingIn, setSigningIn] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const res = await fetch("/api/auth/me", { cache: "no-store" });
      const json = await res.json();
      setUser(json.ok ? json.data.user : null);
    } catch {
      setUser(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const signIn = useCallback(async (): Promise<boolean> => {
    setError(null);
    if (!publicKey || !signMessage) {
      setError("Connect a wallet that supports message signing first.");
      return false;
    }
    setSigningIn(true);
    try {
      const wallet = publicKey.toBase58();
      // 1. Get a challenge.
      const challengeRes = await fetch("/api/auth/challenge", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ wallet }),
      });
      const challenge = await challengeRes.json();
      if (!challenge.ok) throw new Error(challenge.error || "Challenge failed");
      const { message, nonce } = challenge.data;

      // 2. Sign it with the wallet.
      const signature = await signMessage(new TextEncoder().encode(message));
      const signatureB58 = bs58.encode(signature);

      // 3. Verify + create session.
      const verifyRes = await fetch("/api/auth/verify", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ wallet, signature: signatureB58, nonce, message }),
      });
      const verify = await verifyRes.json();
      if (!verify.ok) throw new Error(verify.error || "Verification failed");
      setUser(verify.data.user);
      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : "Sign-in failed");
      return false;
    } finally {
      setSigningIn(false);
    }
  }, [publicKey, signMessage]);

  const signOut = useCallback(async () => {
    await fetch("/api/auth/logout", { method: "POST" });
    setUser(null);
    try {
      await disconnect();
    } catch {
      /* wallet already disconnected */
    }
  }, [disconnect]);

  // If the wallet disconnects, clear the local user (session cookie is cleared
  // lazily on next protected action / explicit sign-out).
  useEffect(() => {
    if (!connected && user) {
      setUser(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connected]);

  const value = useMemo<AuthState>(
    () => ({ user, loading, signingIn, error, signIn, signOut, refresh }),
    [user, loading, signingIn, error, signIn, signOut, refresh],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within <AuthProvider>");
  return ctx;
}
