mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{
        create_funded_user, derive_vote_position, fetch_app, fetch_token_amount,
        fetch_vote_position, fund_token_account, send, set_app_vote_accumulator, setup_with_app,
        vote_ix, warp_forward, withdraw_vote_ix, Env,
    },
    litesvm::LiteSVM,
    nebulous_world::{
        constants::{REWARD_PRECISION, UNSTAKE_FEE_DECAY_SECONDS},
        unstake_fee::{linear_decay_fee_bps, unstake_fee},
    },
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_wvote_test_app_000001";

/// The fee charged on withdrawing `amount` at elapsed=0 — i.e. when the
/// withdrawal lands in the same LiteSVM instance as the vote that opened the
/// position, with no explicit warp, so the full 1% (100 bps) applies.
fn fee_at_elapsed_zero(amount: u64) -> u64 {
    unstake_fee(amount, linear_decay_fee_bps(0)).unwrap()
}

/// Common fixture: registers an app, funds a fresh user's wallet with vote
/// tokens, and votes `initial_stake` in to create a `VotePosition`. Returns
/// everything a `withdraw_vote` test needs.
fn setup_with_position(
    initial_stake: u64,
    wallet_amount: u64,
) -> (LiteSVM, Env, Pubkey, Keypair, Pubkey, Pubkey) {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, wallet_amount);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("initial vote must succeed in test setup");

    (svm, env, app, user, user_token_account, position)
}

/// Even on a full withdrawal (the last stake this `user` holds), the
/// elapsed=0 1% unstake fee is still charged and paid straight to the
/// admin's token account — unlike the old reward-pool redistribution this
/// replaced, there's no "pool would be empty" waiver: the fee doesn't
/// depend on anyone remaining staked to receive it.
#[test]
fn test_withdraw_vote_full_withdrawal_returns_principal_and_zeroes_position() {
    let initial_stake = 4_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote transaction failed");

    let fee = fee_at_elapsed_zero(initial_stake);
    assert!(
        fee > 0,
        "test is only meaningful if a nonzero fee was actually charged"
    );

    let position_account = fetch_vote_position(&svm, position);
    assert_eq!(position_account.amount, 0);
    assert_eq!(position_account.reward_debt, 0);

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_vote_stake, 0);
    // The fee no longer touches the accumulator at all — it went straight
    // to the admin instead.
    assert_eq!(app_account.vote_acc_reward_per_share, 0);

    assert_eq!(fetch_token_amount(&svm, env.vault), 0);
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - fee
    );
}

/// A partial withdrawal (`user` still holds stake afterward) at elapsed=0
/// charges the full 1% unstake fee — paid straight to the admin's token
/// account, not redistributed back into `user`'s own remaining position or
/// anyone else's.
#[test]
fn test_withdraw_vote_partial_withdrawal_leaves_remaining_stake() {
    let initial_stake = 4_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    let withdraw_amount = 1_500u64;
    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        withdraw_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote transaction failed");

    let remaining = initial_stake - withdraw_amount;
    let fee = fee_at_elapsed_zero(withdraw_amount);
    let net_withdraw_amount = withdraw_amount - fee;

    assert_eq!(fetch_vote_position(&svm, position).amount, remaining);

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_vote_stake, remaining);
    // The accumulator is untouched — the fee never goes through it anymore.
    assert_eq!(app_account.vote_acc_reward_per_share, 0);

    // The fee portion of `withdraw_amount` left the vault for the admin,
    // same as the rest of the withdrawal — nothing stays behind.
    assert_eq!(fetch_token_amount(&svm, env.vault), remaining);
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - initial_stake + net_withdraw_amount
    );
}

#[test]
fn test_withdraw_vote_rejects_zero_amount() {
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(4_000, 10_000);

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        0,
    );

    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected withdraw_vote to reject a zero amount, but it succeeded"
    );
}

#[test]
fn test_withdraw_vote_rejects_amount_exceeding_stake() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake + 1,
    );

    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected withdraw_vote to reject an over-withdrawal, but it succeeded"
    );

    // Nothing moved: the position and vault are untouched.
    assert_eq!(fetch_vote_position(&svm, position).amount, initial_stake);
    assert_eq!(fetch_token_amount(&svm, env.vault), initial_stake);
}

/// Exercises the reward-payout CPI leg of `withdraw_vote()` end-to-end on a
/// PARTIAL withdrawal (not just a full one) — the highest-risk path (the
/// `config` PDA signing two separate transfers out of the single global
/// vault in the same instruction: the pending reward, then the returned
/// principal). Mirrors `test_vote_pays_out_pending_reward_on_second_vote` in
/// `test_vote.rs`: manually bumps the app's accumulator (standing in for
/// `fund_app_rewards`) and tops up the vault with extra "reward" balance on
/// top of the principal it already holds, then withdraws part of the stake
/// and asserts both the pending reward AND the principal actually land in
/// the user's wallet, with the position's `reward_debt` re-checkpointed
/// against the new (smaller) remaining amount.
#[test]
fn test_withdraw_vote_pays_out_pending_reward_on_partial_withdrawal() {
    let initial_stake = 1_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    // Stand in for `fund_app_rewards`: bump the accumulator to 1 reward
    // token per staked token, and top up the vault (which already holds
    // `initial_stake` in principal) with extra balance so the payout CPI has
    // something to actually transfer.
    let acc_reward_per_share = REWARD_PRECISION; // 1.0 reward token per staked token
    set_app_vote_accumulator(&mut svm, app, acc_reward_per_share);
    let reward_topup = 50_000u64;
    fund_token_account(
        &mut svm,
        env.vault,
        env.vote_mint,
        env.config,
        initial_stake + reward_topup,
    );

    let withdraw_amount = 400u64;

    // settle_pending(1_000, reward_debt=0, acc=1*PRECISION) = 1_000
    let expected_pending = 1_000u64;

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        withdraw_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote transaction failed");

    let position_account = fetch_vote_position(&svm, position);
    let remaining = initial_stake - withdraw_amount;
    assert_eq!(position_account.amount, remaining);
    // reward_debt_for(remaining, 1*PRECISION) = remaining — checkpointed
    // against the accumulator's value BEFORE this withdrawal's own fee-funding
    // bump (see withdraw_vote's doc comment on why that ordering is correct):
    // the manually-set 1.0-per-share accumulator, unaffected by the fee this
    // same instruction bumps it by afterward.
    assert_eq!(position_account.reward_debt, remaining as u128);

    let fee = fee_at_elapsed_zero(withdraw_amount);
    let net_withdraw_amount = withdraw_amount - fee;

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_vote_stake, remaining);
    // The manually-set accumulator is untouched — the fee no longer bumps it.
    assert_eq!(app_account.vote_acc_reward_per_share, acc_reward_per_share);

    // User received the withdrawn principal (net of the unstake fee) and the
    // pending reward.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - initial_stake + net_withdraw_amount + expected_pending
    );
    // The admin received the fee directly.
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);

    // The single global vault: held (initial_stake + reward_topup) before
    // this instruction, paid out `expected_pending`, the fee (now to the
    // admin), and the net withdrawal — nothing stays behind.
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        initial_stake + reward_topup - expected_pending - withdraw_amount
    );
}

/// Once `UNSTAKE_FEE_DECAY_SECONDS` (a week) has elapsed since a position's
/// `staked_at` checkpoint, the fee is exactly 0 — a genuinely time-decayed
/// case, distinct from the "last staker" waiver the full-withdrawal test
/// above exercises (this is a PARTIAL withdrawal that leaves stake behind,
/// so the pool is never empty; the fee is 0 purely because enough time has
/// passed, not because there's nobody to fund).
#[test]
fn test_withdraw_vote_fee_decays_to_zero_after_the_decay_window() {
    let initial_stake = 4_000u64;
    let wallet_amount = 10_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, wallet_amount);

    warp_forward(&mut svm, UNSTAKE_FEE_DECAY_SECONDS);

    let withdraw_amount = 1_500u64;
    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        withdraw_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote transaction failed");

    let remaining = initial_stake - withdraw_amount;
    assert_eq!(fetch_vote_position(&svm, position).amount, remaining);

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_vote_stake, remaining);
    assert_eq!(app_account.vote_acc_reward_per_share, 0);

    // Full withdraw_amount returned, fee-free — nothing paid to the admin.
    assert_eq!(fetch_token_amount(&svm, env.vault), remaining);
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), 0);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        wallet_amount - initial_stake + withdraw_amount
    );
}

/// The unstake fee is a straight treasury skim, paid directly to the admin's
/// token account — NOT redistributed to other stakers the way vote/tags
/// rewards are. This test proves both halves at once: user A fully exits and
/// pays a fee that lands in `admin_token_account`; user B, who never
/// withdraws, is completely unaffected — their accumulator/reward_debt stay
/// exactly what they were before A's withdrawal, proving A's fee never
/// touched the shared pool at all.
#[test]
fn test_withdraw_vote_fee_is_paid_directly_to_admin_not_other_stakers() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);

    let (user_a, a_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let a_position = derive_vote_position(&env.program_id, &app, &user_a.pubkey());
    let (user_b, b_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let b_position = derive_vote_position(&env.program_id, &app, &user_b.pubkey());

    let a_amount = 4_000u64;
    let b_amount = 5_000u64;
    for (position, token_account, user, amount) in [
        (a_position, a_token_account, &user_a, a_amount),
        (b_position, b_token_account, &user_b, b_amount),
    ] {
        let ix = vote_ix(
            &env,
            &app,
            &position,
            &token_account,
            &user.pubkey(),
            amount,
        );
        send(&mut svm, ix, &user.pubkey(), &[user]).expect("vote must succeed in test setup");
    }

    // User A fully exits at elapsed=0 (full 1% fee); User B stays staked
    // throughout and never touches their own position.
    let ix = withdraw_vote_ix(
        &env,
        &app,
        &a_position,
        &a_token_account,
        &user_a.pubkey(),
        a_amount,
    );
    send(&mut svm, ix, &user_a.pubkey(), &[&user_a]).expect("withdraw_vote must succeed");

    let fee = fee_at_elapsed_zero(a_amount);
    assert!(
        fee > 0,
        "test is only meaningful if a nonzero fee was actually charged"
    );

    // The fee landed directly in the admin's token account.
    assert_eq!(fetch_token_amount(&svm, env.admin_token_account), fee);

    let app_account = fetch_app(&svm, app);
    assert_eq!(app_account.total_vote_stake, b_amount);
    // The shared accumulator never moved — A's fee never touched it.
    assert_eq!(app_account.vote_acc_reward_per_share, 0);

    // User B's position is byte-for-byte what it was after their own vote:
    // no pending reward accrued from A's fee, because there is none.
    let b_position_account = fetch_vote_position(&svm, b_position);
    assert_eq!(b_position_account.amount, b_amount);
    assert_eq!(b_position_account.reward_debt, 0);
    assert_eq!(
        fetch_token_amount(&svm, b_token_account),
        10_000 - b_amount,
        "B's balance is untouched by A's withdrawal"
    );
}
