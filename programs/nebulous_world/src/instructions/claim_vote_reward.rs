use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{APP_SEED, CONFIG_SEED, VOTE_POSITION_SEED};
use crate::reward_math::{reward_debt_for, settle_pending, transfer_from_vault};
use crate::state::{AppAccount, Config, VotePosition};

/// The read/settle half of `withdraw_vote`'s accounts, without the
/// principal-out parts: this instruction never touches `position.amount`.
#[derive(Accounts)]
pub struct ClaimVoteReward<'info> {
    // Not `mut`: this instruction only ever READS `app.vote_acc_reward_per_share`
    // (via `settle_pending`/`reward_debt_for`) — `position.reward_debt` is
    // the only field that changes. Write-locking the single per-app
    // `AppAccount` PDA on every claim would needlessly serialize concurrent
    // claims from different stakers against Solana's parallel-execution
    // scheduler, on what's expected to be the highest-frequency instruction
    // in this set.
    #[account(
        seeds = [APP_SEED, app.app_id.as_bytes()],
        bump = app.bump,
    )]
    pub app: Account<'info, AppAccount>,
    // As in `WithdrawVote`, a claim can only ever target a position that
    // already exists, so this is a plain `mut` + seeds/bump constraint
    // rather than `init_if_needed`. The single `user: Signer` re-derivation
    // of this PDA *is* the ownership check, exactly as documented on
    // `WithdrawVote`.
    #[account(
        mut,
        seeds = [
            VOTE_POSITION_SEED,
            app.key().as_ref(),
            user.key().as_ref(),
        ],
        bump = position.bump,
    )]
    pub position: Account<'info, VotePosition>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = config,
    )]
    pub vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = config.vote_mint,
        associated_token::authority = user,
    )]
    pub user_token_account: Account<'info, TokenAccount>,
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

/// Pays the caller their pending vote-pool reward without touching their
/// staked principal (`position.amount` is read but never reassigned below —
/// that is the entire point of this instruction, as opposed to
/// `withdraw_vote`, which settles-and-pays the same way but then also moves
/// principal).
///
/// A zero-`pending` claim is allowed to go through as a harmless no-op
/// rather than being rejected with a `require!`: `transfer_from_vault`
/// already no-ops on `amount == 0` (skipping the CPI entirely), so the only
/// cost of not guarding here is a slightly wasted transaction — the same
/// trade-off `vote()`/`withdraw_vote()` already accept for their own
/// pending-reward legs.
pub fn handler(ctx: Context<ClaimVoteReward>) -> Result<()> {
    let pending = settle_pending(
        ctx.accounts.position.amount,
        ctx.accounts.position.reward_debt,
        ctx.accounts.app.vote_acc_reward_per_share,
    )?;

    transfer_from_vault(
        &ctx.accounts.vault,
        &ctx.accounts.user_token_account,
        &ctx.accounts.config.to_account_info(),
        ctx.accounts.config.bump,
        &ctx.accounts.token_program,
        pending,
    )?;

    let app = &ctx.accounts.app;
    let position = &mut ctx.accounts.position;
    position.reward_debt = reward_debt_for(position.amount, app.vote_acc_reward_per_share)?;

    Ok(())
}
