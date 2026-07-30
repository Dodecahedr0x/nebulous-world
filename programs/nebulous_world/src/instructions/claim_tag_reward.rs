use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{APP_SEED, APP_TAG_STAKE_SEED, CONFIG_SEED, STAKE_POSITION_SEED};
use crate::error::ErrorCode;
use crate::reward_math::{reward_debt_for, settle_pending, transfer_from_vault};
use crate::state::{AppAccount, AppTagStake, Config, StakePosition};

/// The tags-pool mirror of `ClaimVoteReward`: pays out a staker's pending
/// tags-pool reward without touching their staked principal.
///
/// Takes the exact same `(app, app_tag_stake)` account pair as
/// `StakeTag`/`WithdrawTagStake`, so it carries the exact same fund-drain
/// risk those two instructions were fixed for (see the `constraint =`
/// below): without pinning `app_tag_stake.app == app.key()`, an attacker
/// could pair their OWN cheap `app_tag_stake`/`position` with a victim's
/// well-funded `app` and drain the shared vault via `settle_pending` against
/// the victim's `tags_acc_reward_per_share`.
#[derive(Accounts)]
pub struct ClaimTagReward<'info> {
    // Not `mut`: as in `ClaimVoteReward`, this instruction only ever READS
    // `app.tags_acc_reward_per_share` — `position.reward_debt` is the only
    // field that changes. Write-locking the single per-app `AppAccount` PDA
    // on every claim would needlessly serialize concurrent claims from
    // different stakers/tags against Solana's parallel-execution scheduler.
    #[account(
        seeds = [
            APP_SEED,
            app.app_id.as_bytes(),
        ],
        bump = app.bump
    )]
    pub app: Account<'info, AppAccount>,
    // Not `mut` either: this instruction never touches
    // `app_tag_stake.stake_amount` (or any other field) — `app_tag_stake.app`/
    // `tag`/`bump` are only read, for the ownership check below and for
    // `position`'s seeds/PDA derivation.
    #[account(
        seeds = [
            APP_TAG_STAKE_SEED,
            app_tag_stake.app.as_ref(),
            app_tag_stake.tag.as_ref(),
        ],
        bump = app_tag_stake.bump,
        has_one = app @ ErrorCode::AppTagStakeMismatch,
    )]
    pub app_tag_stake: Account<'info, AppTagStake>,
    // As in `ClaimVoteReward`/`WithdrawTagStake`, a claim can only ever
    // target a position that already exists, so this is a plain `mut` +
    // seeds/bump constraint rather than `init_if_needed`. The single
    // `user: Signer` re-derivation of this PDA *is* the ownership check.
    #[account(
        mut,
        seeds = [
            STAKE_POSITION_SEED,
            app_tag_stake.key().as_ref(),
            user.key().as_ref(),
        ],
        bump = position.bump,
    )]
    pub position: Account<'info, StakePosition>,
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

/// Pays the caller their pending tags-pool reward without touching their
/// staked principal (`position.amount` is read but never reassigned below).
///
/// A zero-`pending` claim is allowed to go through as a harmless no-op
/// rather than being rejected with a `require!`, matching
/// `claim_vote_reward()`'s established precedent.
pub fn handler(ctx: Context<ClaimTagReward>) -> Result<()> {
    let pending = settle_pending(
        ctx.accounts.position.amount,
        ctx.accounts.position.reward_debt,
        ctx.accounts.app.tags_acc_reward_per_share,
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
    position.reward_debt = reward_debt_for(position.amount, app.tags_acc_reward_per_share)?;

    Ok(())
}
