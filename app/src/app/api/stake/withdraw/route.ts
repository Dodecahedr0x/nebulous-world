import { NextRequest } from "next/server";
import { handler, ok, requireUser } from "@/lib/api";
import { unstakeSchema } from "@/lib/validation";
import { withdrawStake } from "@/lib/indexerClient";

// POST /api/stake/withdraw — withdraw one of your active stakes.
//
// In on-chain mode the treasury would return the tokens via the program; here
// we mark the stake inactive so it stops earning revenue and boosting rank.
export const POST = handler(async (req: NextRequest) => {
  const user = await requireUser();
  const body = unstakeSchema.parse(await req.json());
  const result = await withdrawStake(body.stakeId, user.id);
  return ok(result);
});
