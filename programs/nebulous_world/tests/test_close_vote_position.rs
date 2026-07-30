mod common;

use {
    anchor_lang::{solana_program::pubkey::Pubkey, AccountSerialize},
    common::{
        close_vote_position_ix, create_funded_user, derive_vote_position, fetch_vote_position,
        send, setup_with_app, vote_ix, withdraw_vote_ix, Env, AIRDROP_LAMPORTS,
    },
    litesvm::LiteSVM,
    solana_account::Account,
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_close_test_app_000001";

/// Common fixture: registers an app, funds a fresh user's wallet with vote
/// tokens, and votes `initial_stake` in to create a `VotePosition` — the
/// user is both the position's owner and (via `Vote`'s `payer = user`) its
/// rent payer, exactly as every real position is created today.
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

/// The core happy path this whole instruction exists for: once a position is
/// fully withdrawn (amount back to 0), its owner can close it and reclaim
/// the rent SOL — refunded to `position.payer` (here, the same wallet that
/// created it), not just handed to whoever submits the transaction.
#[test]
fn test_close_vote_position_reclaims_rent_for_payer() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote must succeed");
    assert_eq!(fetch_vote_position(&svm, position).amount, 0);

    let position_rent = svm.get_account(&position).unwrap().lamports;
    let payer_balance_before = svm.get_account(&user.pubkey()).unwrap().lamports;

    let ix = close_vote_position_ix(&env.program_id, &position, &user.pubkey(), &user.pubkey());
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("close_vote_position transaction failed");

    // The account is gone entirely — Anchor's `close` zeroes its data and
    // reassigns it to the System Program with 0 lamports, and LiteSVM (like
    // real validators) prunes zero-lamport accounts, so a fresh fetch
    // returns nothing.
    assert!(
        svm.get_account(&position).map(|a| a.lamports).unwrap_or(0) == 0,
        "closed position account should hold no lamports"
    );

    // `user` is both signer (paying this tx's own fee) and the stored
    // `payer` receiving the refund, so assert the net effect: their balance
    // rose by (rent reclaimed - this transaction's own fee), i.e. it did NOT
    // simply drop by the fee alone the way an unrelated tx would.
    let payer_balance_after = svm.get_account(&user.pubkey()).unwrap().lamports;
    assert!(
        payer_balance_after + 5_000 >= payer_balance_before + position_rent,
        "expected the reclaimed rent ({position_rent}) to reach the payer net of tx fees: before={payer_balance_before}, after={payer_balance_after}"
    );
}

/// Closing to a DIFFERENT wallet than the depositor's own — proves the rent
/// genuinely follows `position.payer`, not just "whoever happens to be both
/// owner and payer" (the only case the test above can distinguish).
#[test]
fn test_close_vote_position_refunds_a_third_party_payer_account() {
    // This test constructs the position by hand (bypassing `vote_ix`, which
    // always pays via the depositing `user`) purely to prove the payer
    // constraint reads `position.payer`, not `user`/the tx fee-payer.
    let (mut svm, _deployer, env, app) = setup_with_app(APP_ID);
    let (user, _user_token_account) = create_funded_user(&mut svm, &env, 10_000);

    let (position, bump) = Pubkey::find_program_address(
        &[
            nebulous_world::constants::VOTE_POSITION_SEED,
            app.as_ref(),
            user.pubkey().as_ref(),
        ],
        &env.program_id,
    );

    // A separate wallet, never used to sign anything — just the account
    // `position.payer` will point at.
    let third_party_payer = Pubkey::new_unique();
    svm.airdrop(&third_party_payer, 1).unwrap(); // must exist as a System-owned account

    let rent = svm.minimum_balance_for_rent_exemption(8 + nebulous_world::VotePosition::SPACE);
    let vote_position = nebulous_world::VotePosition {
        app,
        owner: user.pubkey(),
        payer: third_party_payer,
        amount: 0,
        reward_debt: 0,
        staked_at: 0,
        bump,
    };
    let mut data = Vec::new();
    vote_position.try_serialize(&mut data).unwrap();
    svm.set_account(
        position,
        Account {
            lamports: rent,
            data,
            owner: env.program_id,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();

    let third_party_balance_before = svm.get_account(&third_party_payer).unwrap().lamports;

    let ix = close_vote_position_ix(
        &env.program_id,
        &position,
        &third_party_payer,
        &user.pubkey(),
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("close_vote_position transaction failed");

    let third_party_balance_after = svm.get_account(&third_party_payer).unwrap().lamports;
    assert_eq!(
        third_party_balance_after,
        third_party_balance_before + rent,
        "the third-party payer, not the signer `user`, must receive the full rent refund"
    );
}

#[test]
fn test_close_vote_position_rejects_nonzero_stake() {
    let initial_stake = 4_000u64;
    let (mut svm, env, _app, user, _user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = close_vote_position_ix(&env.program_id, &position, &user.pubkey(), &user.pubkey());
    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected close_vote_position to reject a position that still holds stake"
    );

    // Nothing changed: the position is untouched.
    assert_eq!(fetch_vote_position(&svm, position).amount, initial_stake);
}

/// Passing an account that doesn't match `position.payer` must be rejected
/// outright — proves the refund destination can't be redirected by whoever
/// happens to submit the close transaction.
#[test]
fn test_close_vote_position_rejects_wrong_payer_account() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote must succeed");

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), AIRDROP_LAMPORTS).unwrap();

    let ix = close_vote_position_ix(
        &env.program_id,
        &position,
        &attacker.pubkey(),
        &user.pubkey(),
    );
    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected close_vote_position to reject a payer account that isn't position.payer"
    );

    // Nothing changed: the position still exists, untouched.
    assert_eq!(fetch_vote_position(&svm, position).amount, 0);
}

/// A signer who isn't this position's owner can't close it at all: `user`'s
/// pubkey is one of the seeds that derives `position`'s own address, so a
/// different signer simply can't produce a valid `(position, user)` pair
/// pointing at somebody else's account.
#[test]
fn test_close_vote_position_rejects_non_owner_signer() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_vote_ix(
        &env,
        &app,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_vote must succeed");

    let not_the_owner = Keypair::new();
    svm.airdrop(&not_the_owner.pubkey(), AIRDROP_LAMPORTS)
        .unwrap();

    // Same `position`/`payer` accounts, but signed and "user"-attributed to
    // an unrelated wallet — the seeds re-derivation must fail.
    let ix = close_vote_position_ix(
        &env.program_id,
        &position,
        &user.pubkey(),
        &not_the_owner.pubkey(),
    );
    assert!(
        send(&mut svm, ix, &not_the_owner.pubkey(), &[&not_the_owner]).is_err(),
        "expected close_vote_position to reject a signer who isn't the position's owner"
    );
}
