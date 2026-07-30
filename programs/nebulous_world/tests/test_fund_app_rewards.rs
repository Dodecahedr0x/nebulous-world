mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{
        create_funded_user, derive_vote_position, fetch_app, fetch_token_amount,
        fund_app_rewards_ix, fund_token_account, send, setup_with_app, vote_ix, Env,
    },
    litesvm::LiteSVM,
    nebulous_world::{constants::REWARD_PRECISION, RewardPool},
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_fund_test_app_0000001";

/// The funder is always `Config.authority` (the deployer), and the only token
/// account it can spend from is its own ATA — `env.admin_token_account`.
/// Returns it, holding `amount`.
fn funded_admin_wallet(svm: &mut LiteSVM, env: &Env, deployer: &Keypair, amount: u64) -> Pubkey {
    fund_token_account(
        svm,
        env.admin_token_account,
        env.vote_mint,
        deployer.pubkey(),
        amount,
    );
    env.admin_token_account
}

/// Votes `amount` from a fresh user, so the vote pool has stakers — an empty
/// pool (total_vote_stake == 0) can't be funded at all (see
/// `test_fund_app_rewards_rejects_zero_total_stake`).
fn add_voter(svm: &mut LiteSVM, env: &Env, app: &Pubkey, amount: u64) {
    let (voter, voter_token_account) = create_funded_user(svm, env, 10_000);
    let position = derive_vote_position(&env.program_id, app, &voter.pubkey());
    let ix = vote_ix(
        env,
        app,
        &position,
        &voter_token_account,
        &voter.pubkey(),
        amount,
    );
    send(svm, ix, &voter.pubkey(), &[&voter]).expect("setup vote must succeed");
}

#[test]
fn test_fund_app_rewards_bumps_accumulator_and_transfers_tokens() {
    let (mut svm, deployer, env, app) = setup_with_app(APP_ID);

    let total_vote_stake = 1_000u64;
    add_voter(&mut svm, &env, &app, total_vote_stake);

    // The vote's principal already sits in the single global vault, so
    // capture the vault balance here rather than assuming it starts at 0 —
    // unlike the old per-app reward vault, this vault is shared with
    // vote/tag-stake principal.
    let vault_before_funding = fetch_token_amount(&svm, env.vault);
    assert_eq!(vault_before_funding, total_vote_stake);

    // Fund the vote pool with 500 real tokens from the deployer (who is
    // `Config.authority`).
    let funder_token_account = funded_admin_wallet(&mut svm, &env, &deployer, 20_000);

    let fund_amount = 500u64;
    let ix = fund_app_rewards_ix(
        &env,
        &app,
        &funder_token_account,
        &deployer.pubkey(),
        RewardPool::Vote,
        fund_amount,
    );
    send(&mut svm, ix, &deployer.pubkey(), &[&deployer])
        .expect("fund_app_rewards transaction failed");

    // Tokens actually moved: vault gained `fund_amount` on top of the
    // pre-existing principal, funder lost it.
    assert_eq!(
        fetch_token_amount(&svm, env.vault),
        vault_before_funding + fund_amount
    );
    assert_eq!(
        fetch_token_amount(&svm, funder_token_account),
        20_000 - fund_amount
    );

    // Accumulator bumped by exactly fund_amount * PRECISION / total_vote_stake.
    let app_account = fetch_app(&svm, app);
    let expected_delta = (fund_amount as u128) * REWARD_PRECISION / total_vote_stake as u128;
    assert_eq!(app_account.vote_acc_reward_per_share, expected_delta);
    // The tags pool must be untouched by a Vote-pool funding call.
    assert_eq!(app_account.tags_acc_reward_per_share, 0);
}

#[test]
fn test_fund_app_rewards_rejects_non_authority_signer() {
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);

    // Stake something so the ONLY possible failure reason is the authority
    // mismatch, not `NoStakers`.
    add_voter(&mut svm, &env, &app, 1_000);
    let vault_before = fetch_token_amount(&svm, env.vault);

    // A stranger, unrelated to `Config.authority`, tries to fund the pool.
    let (stranger, stranger_token_account) = create_funded_user(&mut svm, &env, 20_000);

    let ix = fund_app_rewards_ix(
        &env,
        &app,
        &stranger_token_account,
        &stranger.pubkey(),
        RewardPool::Vote,
        500,
    );
    assert!(
        send(&mut svm, ix, &stranger.pubkey(), &[&stranger]).is_err(),
        "expected fund_app_rewards to reject a non-authority signer, but it succeeded"
    );

    // Nothing moved.
    assert_eq!(fetch_token_amount(&svm, env.vault), vault_before);
    assert_eq!(fetch_token_amount(&svm, stranger_token_account), 20_000);
}

#[test]
fn test_fund_app_rewards_rejects_zero_total_stake() {
    let (mut svm, deployer, env, app) = setup_with_app(APP_ID);

    // Nobody has ever voted: total_vote_stake == 0.
    let funder_token_account = funded_admin_wallet(&mut svm, &env, &deployer, 20_000);

    let ix = fund_app_rewards_ix(
        &env,
        &app,
        &funder_token_account,
        &deployer.pubkey(),
        RewardPool::Vote,
        500,
    );
    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected fund_app_rewards to reject funding an empty pool, but it succeeded"
    );
    assert_eq!(fetch_token_amount(&svm, funder_token_account), 20_000);
}

#[test]
fn test_fund_app_rewards_rejects_zero_total_stake_tags_pool() {
    // Nobody has staked any tag for this app yet (`stake_tag` was never
    // called), so `total_tag_stake` is still 0 — this documents/locks in
    // that a Tags-pool funding attempt correctly hits the same `NoStakers`
    // guard as the vote pool.
    let (mut svm, deployer, env, app) = setup_with_app(APP_ID);

    let funder_token_account = funded_admin_wallet(&mut svm, &env, &deployer, 20_000);

    let ix = fund_app_rewards_ix(
        &env,
        &app,
        &funder_token_account,
        &deployer.pubkey(),
        RewardPool::Tags,
        500,
    );
    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected fund_app_rewards to reject funding an empty tags pool, but it succeeded"
    );
}

#[test]
fn test_fund_app_rewards_rejects_zero_amount() {
    let (mut svm, deployer, env, app) = setup_with_app(APP_ID);

    add_voter(&mut svm, &env, &app, 1_000);
    let funder_token_account = funded_admin_wallet(&mut svm, &env, &deployer, 20_000);

    let ix = fund_app_rewards_ix(
        &env,
        &app,
        &funder_token_account,
        &deployer.pubkey(),
        RewardPool::Vote,
        0,
    );
    assert!(
        send(&mut svm, ix, &deployer.pubkey(), &[&deployer]).is_err(),
        "expected fund_app_rewards to reject a zero amount, but it succeeded"
    );
}
