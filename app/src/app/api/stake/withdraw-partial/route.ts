import { NextRequest } from "next/server";
import { handler, ok, requireUser } from "@/lib/api";
import { unstakePartialSchema } from "@/lib/validation";
import { withdrawPartialStakes } from "@/lib/indexerClient";

// POST /api/stake/withdraw-partial — withdraw `amount` (up to the full
// active total) off one app-tag's stake, possibly spanning more than one
// active row (a user can stake on the same tag more than once over time,
// each adding to the single on-chain StakePosition — see
// indexer/src/handlers/stakes.rs's withdraw_partial). Used by the profile
// page's "Your stakes" unstake action, which withdraws this same amount
// on-chain.
export const POST = handler(async (req: NextRequest) => {
  const user = await requireUser();
  const body = unstakePartialSchema.parse(await req.json());
  const result = await withdrawPartialStakes(body.appTagId, body.amount, user.id);
  return ok(result);
});
