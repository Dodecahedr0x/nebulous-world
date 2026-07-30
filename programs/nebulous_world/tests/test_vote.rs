mod common;

use {
    common::{
        create_funded_user, derive_vote_position, fetch_app, fetch_token_amount,
        fetch_vote_position, fund_token_account, send, set_app_vote_accumulator, setup_with_app,
        vote_ix, warp_forward,
    },
    nebulous_world::constants::REWARD_PRECISION,
    solana_clock::Clock,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_vote_test_app_0000001";

#[test]
fn test_vote_locks_principal_and_creates_position() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    let amount = 4_000u64;
    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("vote transaction failed");

    // The position was created with the right app/owner/amount/reward_debt.
    let position_account = fetch_vote_position(&svm, position);
    assert_eq!(position_account.app, app);
    assert_eq!(position_account.owner, user.pubkey());
    assert_eq!(position_account.amount, amount);
    // No rewards were ever funded, so the accumulator is still 0 and the
    // fresh checkpoint must be 0 too.
    assert_eq!(position_account.reward_debt, 0);
    // A brand-new position's staked_at is exactly `now` (weighted_avg_timestamp
    // collapses to `now` when the old amount is 0 — see unstake_fee.rs) —
    // matches the LiteSVM instance's current clock, which this test never
    // advances.
    assert_eq!(
        position_account.staked_at,
        svm.get_sysvar::<Clock>().unix_timestamp
    );

    // The app's total_vote_stake reflects the new stake.
    assert_eq!(fetch_app(&svm, app).total_vote_stake, amount);

    // Tokens actually moved: the single global vault gained `amount`, user
    // lost `amount`.
    assert_eq!(fetch_token_amount(&svm, env.vault), amount);
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        10_000 - amount
    );
}

#[test]
fn test_vote_rejects_zero_amount() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        0,
    );

    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected vote to reject a zero amount, but it succeeded"
    );
}

#[test]
fn test_vote_accumulates_across_two_deposits() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    for amount in [1_000u64, 2_500u64] {
        let ix = vote_ix(
            &env,
            &app,
            &position,
            &user_token_account,
            &user.pubkey(),
            amount,
        );
        send(&mut svm, ix, &user.pubkey(), &[&user]).expect("vote transaction failed");
    }

    assert_eq!(fetch_vote_position(&svm, position).amount, 3_500);
    assert_eq!(fetch_app(&svm, app).total_vote_stake, 3_500);
}

/// Exercises the reward-payout CPI leg of `vote()` end-to-end — the
/// highest-risk path (the `config` PDA actually signing a transfer out of
/// the single global vault), which every other test above never touches
/// since they all run with `vote_acc_reward_per_share == 0` (so
/// `settle_pending` is always 0 and `transfer_from_vault` always hits its
/// no-op early return). This test votes once to create a nonzero position,
/// manually bumps the app's accumulator (standing in for `fund_app_rewards`)
/// and tops up the global vault with extra "reward" balance, then votes
/// again and asserts the pending reward actually lands in the user's wallet
/// and the position's `reward_debt` checkpoints to the new accumulator
/// value.
///
/// Unlike the pre-refactor version of this test (which pre-funded a
/// dedicated `vote_reward_vault` separate from `vote_vault`), there is now
/// only one vault: the "reward top-up" is added directly on top of the
/// principal balance already sitting in `vault` from the first vote.
#[test]
fn test_vote_pays_out_pending_reward_on_second_vote() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 10_000);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    // First vote: creates the position at amount=1_000 with reward_debt=0
    // (accumulator is still 0 at this point).
    let first_amount = 1_000u64;
    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        first_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("first vote must succeed in test setup");

    // Stand in for `fund_app_rewards`: bump the accumulator to 1 reward
    // token per staked token, and top up the vault (which already holds
    // `first_amount` in principal from the vote above) with extra balance so
    // the payout CPI has something to actually transfer.
    let acc_reward_per_share = REWARD_PRECISION; // 1.0 reward token per staked token
    set_app_vote_accumulator(&mut svm, app, acc_reward_per_share);
    let reward_topup = 50_000u64;
    fund_token_account(
        &mut svm,
        env.vault,
        env.vote_mint,
        env.config,
        first_amount + reward_topup,
    );

    // Expected pending reward: settle_pending(1_000, reward_debt=0, acc=1*PRECISION) = 1_000.
    let expected_pending = 1_000u64;

    let second_amount = 500u64;
    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        second_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("second vote transaction failed");

    // The position grew by `second_amount` and checkpointed against the new
    // accumulator: reward_debt_for(1_500, 1*PRECISION) = 1_500.
    let position_account = fetch_vote_position(&svm, position);
    assert_eq!(position_account.amount, first_amount + second_amount);
    assert_eq!(position_account.reward_debt, 1_500);

    // The reward actually landed in the user's wallet: started with 10_000,
    // paid `first_amount` + `second_amount` in principal, received
    // `expected_pending` back as reward.
    assert_eq!(
        fetch_token_amount(&svm, user_token_account),
        10_000 - first_amount - second_amount + expected_pending
    );

    // The single global vault: held (first_amount + reward_topup) before
    // this instruction, paid out `expected_pending`, then received
    // `second_amount` of fresh principal.
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        first_amount + reward_topup - expected_pending + second_amount
    );
}

/// `staked_at` is a size-weighted average across deposits (see
/// `unstake_fee::weighted_avg_timestamp`), not just "timestamp of the first
/// deposit" — a large top-up should pull an old, small position's checkpoint
/// close to the top-up time, not leave it stuck at the original (now stale)
/// checkpoint. This is what closes the exploit a simpler "first deposit
/// only" design would leave open: stake a token once, wait out the fee
/// decay window, then dump an arbitrarily large top-up in and withdraw it
/// immediately fee-free.
#[test]
fn test_vote_staked_at_is_a_weighted_average_across_deposits() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, 1_000_100);
    let position = derive_vote_position(&env.program_id, &app, &user.pubkey());

    // A tiny first deposit...
    let first_amount = 100u64;
    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        first_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("first vote must succeed");
    let staked_at_after_first = fetch_vote_position(&svm, position).staked_at;

    // ...then, a week later, a much larger top-up.
    let elapsed = 7 * 24 * 60 * 60;
    warp_forward(&mut svm, elapsed);
    let second_amount = 1_000_000u64;
    let ix = vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        second_amount,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("second vote must succeed");

    let position_account = fetch_vote_position(&svm, position);
    assert_eq!(position_account.amount, first_amount + second_amount);

    let expected_staked_at = nebulous_world::unstake_fee::weighted_avg_timestamp(
        staked_at_after_first,
        first_amount,
        staked_at_after_first + elapsed,
        second_amount,
    );
    assert_eq!(position_account.staked_at, expected_staked_at);
    // The top-up so vastly outweighs the tiny first deposit (10_000x, i.e.
    // the first deposit is ~0.01% of the new total) that the checkpoint
    // should move at least 99% of the way from the original timestamp
    // toward the top-up's own timestamp, not stay anywhere near the stale
    // original one.
    let moved = position_account.staked_at - staked_at_after_first;
    assert!(
        moved >= elapsed * 99 / 100,
        "a 10_000x top-up should move staked_at at least 99% of the way to the top-up time, \
         moved {moved}s of {elapsed}s"
    );
}
