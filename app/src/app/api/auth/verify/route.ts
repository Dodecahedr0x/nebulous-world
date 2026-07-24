import { NextRequest } from "next/server";
import { handler, ok, ApiError, requireRateLimit, RATE_LIMITS } from "@/lib/api";
import { authVerifySchema } from "@/lib/validation";
import { verifyNonce, issueSessionToken, setSessionCookie } from "@/lib/session";
import { verifySignature, isValidWallet } from "@/lib/solana-auth";
import { connectUser } from "@/lib/indexerClient";

// POST /api/auth/verify — verify the signed challenge and start a session.
// IP-rate-limited, same reasoning as /api/auth/challenge: no session exists
// yet to key on, and this is exactly the endpoint a brute-forced signature
// guess would hit.
export const POST = handler(async (req: NextRequest) => {
  await requireRateLimit(req, RATE_LIMITS.auth);
  const body = authVerifySchema.parse(await req.json());

  if (!isValidWallet(body.wallet)) throw new ApiError("Invalid wallet", 400);

  // The nonce must be present in the signed message and independently valid.
  if (!body.message.includes(body.nonce)) {
    throw new ApiError("Nonce/message mismatch", 400);
  }
  if (!verifyNonce(body.nonce)) {
    throw new ApiError("Challenge expired — please try again", 400);
  }
  if (!verifySignature(body.wallet, body.message, body.signature)) {
    throw new ApiError("Signature verification failed", 401);
  }

  // Upsert the user by wallet — only reachable once the signature above has
  // actually proven ownership of it.
  const user = await connectUser(body.wallet);

  const token = issueSessionToken(user.wallet, user.id);
  await setSessionCookie(token);

  return ok({
    user: { id: user.id, wallet: user.wallet, handle: user.handle },
  });
});
