import { NextRequest } from "next/server";
import { handler, ok, requireUser } from "@/lib/api";
import { unvotePartialSchema } from "@/lib/validation";
import { withdrawPartialVotes } from "@/lib/indexerClient";

// POST /api/vote/withdraw-partial — withdraw `amount` (up to the full
// active total) off one app's vote, possibly spanning more than one active
// row (a user can vote on the same app more than once over time, each
// adding to the single on-chain VotePosition — see indexer/src/handlers/
// votes.rs's withdraw_partial). Used by the profile page's "Your stakes"
// unstake action, which withdraws this same amount on-chain.
export const POST = handler(async (req: NextRequest) => {
  const user = await requireUser();
  const body = unvotePartialSchema.parse(await req.json());
  const result = await withdrawPartialVotes(body.appId, body.amount, user.id);
  return ok(result);
});
