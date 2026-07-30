mod common;

use {
    anchor_lang::solana_program::pubkey::Pubkey,
    common::{initialize_ix, send, set_upgrade_authority, setup_svm_with_mint},
    solana_signer::Signer,
};

#[test]
fn test_initialize() {
    let program_id = nebulous_world::id();
    let (mut svm, payer, vote_mint) = setup_svm_with_mint();
    let program_data = set_upgrade_authority(&mut svm, &program_id, payer.pubkey());

    let ix = initialize_ix(&program_id, &payer.pubkey(), &vote_mint, &program_data, 250);

    send(&mut svm, ix, &payer.pubkey(), &[&payer]).expect("initialize transaction failed");
}

#[test]
fn test_initialize_rejects_fee_above_10_000_bps() {
    let program_id = nebulous_world::id();
    let (mut svm, payer, vote_mint) = setup_svm_with_mint();
    let program_data = set_upgrade_authority(&mut svm, &program_id, payer.pubkey());

    let ix = initialize_ix(
        &program_id,
        &payer.pubkey(),
        &vote_mint,
        &program_data,
        10_001,
    );

    assert!(
        send(&mut svm, ix, &payer.pubkey(), &[&payer]).is_err(),
        "expected initialize to reject a fee > 10_000 bps, but it succeeded"
    );
}

#[test]
fn test_initialize_rejects_non_upgrade_authority_signer() {
    let program_id = nebulous_world::id();
    let (mut svm, payer, vote_mint) = setup_svm_with_mint();
    // Leave the program's upgrade authority as some other, unrelated key —
    // `payer` (who signs the `initialize` call below) is NOT that authority.
    let real_upgrade_authority = Pubkey::new_unique();
    let program_data = set_upgrade_authority(&mut svm, &program_id, real_upgrade_authority);

    let ix = initialize_ix(&program_id, &payer.pubkey(), &vote_mint, &program_data, 250);

    assert!(
        send(&mut svm, ix, &payer.pubkey(), &[&payer]).is_err(),
        "expected initialize to reject a signer that is not the program's upgrade authority, but it succeeded"
    );
}
