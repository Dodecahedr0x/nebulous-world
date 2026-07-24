import { NextRequest } from "next/server";
import { z } from "zod";
import { handler, ok, ApiError, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { createNonce } from "@/lib/session";
import { buildSignInMessage, isValidWallet } from "@/lib/solana-auth";

const schema = z.object({ wallet: z.string().min(32).max(64) });

// POST /api/auth/challenge — issue a signed nonce + the message to sign.
// IP-rate-limited (there's no session yet at this point in the flow) to
// keep this from being spammed to mint/verify signatures at scale.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.auth);
  const body = schema.parse(await req.json());
  if (!isValidWallet(body.wallet)) throw new ApiError("Invalid wallet", 400);

  const nonce = createNonce();
  const issuedAt = new Date().toISOString();
  const message = buildSignInMessage({
    nonce,
    issuedAt,
    statement:
      "Sign in to nebulous.world. This request will not trigger a blockchain transaction or cost any fees.",
  });

  return ok({ message, nonce, issuedAt });
});
