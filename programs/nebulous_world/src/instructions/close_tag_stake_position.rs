use anchor_lang::prelude::*;

use crate::constants::STAKE_POSITION_SEED;
use crate::error::ErrorCode;
use crate::state::StakePosition;

/// The tag-staking mirror of `CloseVotePosition` — see that instruction's
/// doc comments for the reasoning behind every constraint here, identical
/// throughout except the position type and seed.
#[derive(Accounts)]
pub struct CloseTagStakePosition<'info> {
    #[account(
        mut,
        seeds = [
            STAKE_POSITION_SEED,
            position.app_tag_stake.as_ref(),
            user.key().as_ref(),
        ],
        bump = position.bump,
        close = payer,
        has_one = payer,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(mut)]
    pub payer: SystemAccount<'info>,
    pub user: Signer<'info>,
}

pub fn handler(ctx: Context<CloseTagStakePosition>) -> Result<()> {
    require!(ctx.accounts.position.amount == 0, ErrorCode::NonZeroStake);
    Ok(())
}
