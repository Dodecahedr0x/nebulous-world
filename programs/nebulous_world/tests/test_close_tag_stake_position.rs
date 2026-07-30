mod common;

use {
    anchor_lang::{solana_program::pubkey::Pubkey, AccountSerialize},
    common::{
        close_tag_stake_position_ix, create_funded_user, derive_stake_position,
        fetch_stake_position, send, setup_with_tag, stake_tag_ix, withdraw_tag_stake_ix, Env,
        TagPdas, AIRDROP_LAMPORTS,
    },
    litesvm::LiteSVM,
    solana_account::Account,
    solana_keypair::Keypair,
    solana_signer::Signer,
};

const APP_ID: &str = "cid_ctag_close_app_00001";
const TAG_ID: &str = "defi";

/// Common fixture: registers an app + tag, funds a fresh user's wallet, and
/// stakes `initial_stake` in to create a `StakePosition` — the user is both
/// the position's owner and (via `StakeTag`'s `payer = user`) its rent
/// payer, exactly as every real position is created today.
fn setup_with_position(
    initial_stake: u64,
    wallet_amount: u64,
) -> (LiteSVM, Env, Pubkey, TagPdas, Keypair, Pubkey, Pubkey) {
    let (mut svm, _deployer, env, app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);
    let (user, user_token_account) = create_funded_user(&mut svm, &env, wallet_amount);
    let position = derive_stake_position(&env.program_id, &tag_pdas.app_tag_stake, &user.pubkey());

    let ix = stake_tag_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("initial stake_tag must succeed in test setup");

    (svm, env, app, tag_pdas, user, user_token_account, position)
}

/// The core happy path: once a tag-stake position is fully withdrawn, its
/// owner can close it and reclaim the rent SOL, refunded to
/// `position.payer` — here, the same wallet that created it.
#[test]
fn test_close_tag_stake_position_reclaims_rent_for_payer() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake must succeed");
    assert_eq!(fetch_stake_position(&svm, position).amount, 0);

    let position_rent = svm.get_account(&position).unwrap().lamports;
    let payer_balance_before = svm.get_account(&user.pubkey()).unwrap().lamports;

    let ix =
        close_tag_stake_position_ix(&env.program_id, &position, &user.pubkey(), &user.pubkey());
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("close_tag_stake_position transaction failed");

    assert!(
        svm.get_account(&position).map(|a| a.lamports).unwrap_or(0) == 0,
        "closed position account should hold no lamports"
    );
    let payer_balance_after = svm.get_account(&user.pubkey()).unwrap().lamports;
    assert!(
        payer_balance_after + 5_000 >= payer_balance_before + position_rent,
        "expected the reclaimed rent ({position_rent}) to reach the payer net of tx fees: before={payer_balance_before}, after={payer_balance_after}"
    );
}

/// Proves the rent genuinely follows `position.payer`, not just "whoever
/// happens to be both owner and payer" — mirrors
/// `test_close_vote_position_refunds_a_third_party_payer_account`.
#[test]
fn test_close_tag_stake_position_refunds_a_third_party_payer_account() {
    let (mut svm, _deployer, env, _app, tag_pdas) = setup_with_tag(APP_ID, TAG_ID);

    let user = Keypair::new();
    svm.airdrop(&user.pubkey(), AIRDROP_LAMPORTS).unwrap();

    let (position, bump) = Pubkey::find_program_address(
        &[
            nebulous_world::constants::STAKE_POSITION_SEED,
            tag_pdas.app_tag_stake.as_ref(),
            user.pubkey().as_ref(),
        ],
        &env.program_id,
    );

    let third_party_payer = Pubkey::new_unique();
    svm.airdrop(&third_party_payer, 1).unwrap();

    let rent = svm.minimum_balance_for_rent_exemption(8 + nebulous_world::StakePosition::SPACE);
    let stake_position = nebulous_world::StakePosition {
        app_tag_stake: tag_pdas.app_tag_stake,
        owner: user.pubkey(),
        payer: third_party_payer,
        amount: 0,
        reward_debt: 0,
        staked_at: 0,
        bump,
    };
    let mut data = Vec::new();
    stake_position.try_serialize(&mut data).unwrap();
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

    let ix = close_tag_stake_position_ix(
        &env.program_id,
        &position,
        &third_party_payer,
        &user.pubkey(),
    );
    send(&mut svm, ix, &user.pubkey(), &[&user])
        .expect("close_tag_stake_position transaction failed");

    let third_party_balance_after = svm.get_account(&third_party_payer).unwrap().lamports;
    assert_eq!(
        third_party_balance_after,
        third_party_balance_before + rent,
        "the third-party payer, not the signer `user`, must receive the full rent refund"
    );
}

#[test]
fn test_close_tag_stake_position_rejects_nonzero_stake() {
    let initial_stake = 4_000u64;
    let (mut svm, env, _app, _tag_pdas, user, _user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix =
        close_tag_stake_position_ix(&env.program_id, &position, &user.pubkey(), &user.pubkey());
    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected close_tag_stake_position to reject a position that still holds stake"
    );
    assert_eq!(fetch_stake_position(&svm, position).amount, initial_stake);
}

#[test]
fn test_close_tag_stake_position_rejects_wrong_payer_account() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake must succeed");

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), AIRDROP_LAMPORTS).unwrap();

    let ix = close_tag_stake_position_ix(
        &env.program_id,
        &position,
        &attacker.pubkey(),
        &user.pubkey(),
    );
    assert!(
        send(&mut svm, ix, &user.pubkey(), &[&user]).is_err(),
        "expected close_tag_stake_position to reject a payer account that isn't position.payer"
    );
    assert_eq!(fetch_stake_position(&svm, position).amount, 0);
}

#[test]
fn test_close_tag_stake_position_rejects_non_owner_signer() {
    let initial_stake = 4_000u64;
    let (mut svm, env, app, tag_pdas, user, user_token_account, position) =
        setup_with_position(initial_stake, 10_000);

    let ix = withdraw_tag_stake_ix(
        &env,
        &app,
        &tag_pdas,
        &position,
        &user_token_account,
        &user.pubkey(),
        initial_stake,
    );
    send(&mut svm, ix, &user.pubkey(), &[&user]).expect("withdraw_tag_stake must succeed");

    let not_the_owner = Keypair::new();
    svm.airdrop(&not_the_owner.pubkey(), AIRDROP_LAMPORTS)
        .unwrap();

    let ix = close_tag_stake_position_ix(
        &env.program_id,
        &position,
        &user.pubkey(),
        &not_the_owner.pubkey(),
    );
    assert!(
        send(&mut svm, ix, &not_the_owner.pubkey(), &[&not_the_owner]).is_err(),
        "expected close_tag_stake_position to reject a signer who isn't the position's owner"
    );
}
