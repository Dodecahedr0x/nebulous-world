import { NextRequest } from "next/server";
import { handler, ok, requireUser } from "@/lib/api";
import { unvoteSchema } from "@/lib/validation";
import { withdrawVote } from "@/lib/indexerClient";

// POST /api/vote/withdraw — withdraw an active vote.
//
// In on-chain mode the program returns the tokens; here we mark the vote
// inactive so it stops boosting rank and earning revenue.
export const POST = handler(async (req: NextRequest) => {
  const user = await requireUser();
  const body = unvoteSchema.parse(await req.json());
  const result = await withdrawVote(body.voteId, user.id);
  return ok(result);
});
