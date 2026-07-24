"use client";

import { Tooltip } from "@/components/ui/Tooltip";

/**
 * Replaces the old spelled-out "you'd receive ~X, shrinks to 0 over a week"
 * paragraph with just the current percentage plus a "?" explaining the
 * decay — the fee is a straight treasury skim (see withdraw_vote.rs's/
 * withdraw_tag_stake.rs's doc comments: paid directly to `admin_token_account`,
 * never touching the reward accumulator), so the tooltip says exactly that
 * rather than the UI repeating the inaccurate "goes to other stakers" claim
 * that used to live here. The label itself just says "fee", not "unstaking
 * fee" — every place this renders already sits right next to an
 * Unstake/Withdraw button or amount, so spelling that out again is redundant
 * chrome, not information.
 */
export function UnstakeFeeNotice({ feeBps }: { feeBps: number }) {
  if (feeBps <= 0) return null;
  return (
    <span className="inline-flex items-center gap-1 text-xs text-slate-steel">
      {(feeBps / 100).toFixed(2)}% fee
      <Tooltip text="Shrinks to 0% over the first week after staking. Paid to the treasury — never redistributed to other stakers." />
    </span>
  );
}
