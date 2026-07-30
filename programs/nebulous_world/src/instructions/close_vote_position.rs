use anchor_lang::prelude::*;

use crate::constants::VOTE_POSITION_SEED;
use crate::error::ErrorCode;
use crate::state::VotePosition;

#[derive(Accounts)]
pub struct CloseVotePosition<'info> {
    // Self-referencing seeds (re-deriving from `position.app`, already
    // deserialized, rather than requiring a separate `app` account) — the
    // same pattern `StakeTag`/`WithdrawTagStake` already use for
    // `app_tag_stake`'s own seeds. Keeps this instruction's account list
    // minimal, which matters for a UI batching many closes into one
    // transaction (Solana's ~1232-byte tx size limit).
    #[account(
        mut,
        seeds = [
            VOTE_POSITION_SEED,
            position.app.as_ref(),
            user.key().as_ref(),
        ],
        bump = position.bump,
        close = payer,
        has_one = payer,
    )]
    pub position: Account<'info, VotePosition>,
    /// The account that originally paid this position's rent (stored on
    /// `position` at creation — see `vote::handler`), refunded that rent
    /// here on close. Verified against `position.payer` rather than trusted
    /// from the caller, so a permissionless cleanup transaction can never
    /// redirect somebody else's rent refund to itself. Doesn't need to
    /// sign: receiving lamports back requires no authorization from the
    /// receiver.
    #[account(mut)]
    pub payer: SystemAccount<'info>,
    // The single `user: Signer` re-derivation of `position`'s PDA above IS
    // the ownership check — same pattern as `WithdrawVote`/`ClaimVoteReward`.
    pub user: Signer<'info>,
}

/// Closes an emptied `VotePosition`, reclaiming its rent for whoever
/// originally paid it. Only the position's own owner can call this (see
/// `user: Signer` above) — closing is a cleanup convenience for a wallet's
/// own dust, not something a third party can force.
///
/// `amount == 0` alone is sufficient proof there's no unclaimed reward being
/// discarded along with the account: every path that changes `amount` also
/// re-checkpoints `reward_debt` against the SAME accumulator value in the
/// same instruction (`vote`/`withdraw_vote`), and `reward_debt_for(0, _)` is
/// always exactly 0 (see `reward_math.rs`), so a position sitting at
/// `amount == 0` necessarily has `reward_debt == 0` too — nothing pending.
pub fn handler(ctx: Context<CloseVotePosition>) -> Result<()> {
    require!(ctx.accounts.position.amount == 0, ErrorCode::NonZeroStake);
    Ok(())
}
