#![cfg(test)]

extern crate std;

use crate::{FinnanSSCe, FinnanSSCeClient, ProposalStatus};
use soroban_sdk::{
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    Address, Env, String,
};

// Helper: set up the full test environment.
// Deploys a mock USDC SAC, registers the FinnanSSCe
// contract, initializes it, and funds the treasury.
fn setup_env() -> (
    Env,
    FinnanSSCeClient<'static>,
    Address, // admin
    Address, // USDC token address
    TokenClient<'static>,
    StellarAssetClient<'static>,
    Address, // contract address
) {
    let env = Env::default();
    env.mock_all_auths();

    // Treasury admin = SSC Alangilan treasurer.
    let admin = Address::generate(&env);

    // Deploy a Stellar Asset Contract for USDC mock.
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token_address = sac.address();
    let token_client = TokenClient::new(&env, &token_address);
    let token_sac = StellarAssetClient::new(&env, &token_address);

    // Register FinnanSSCe contract.
    let contract_id = env.register(FinnanSSCe, ());
    let client = FinnanSSCeClient::new(&env, &contract_id);

    // Initialize the governance contract.
    client.initialize(&admin, &token_address);

    // Mint 100_000 USDC (7 decimals) to the treasury contract address.
    token_sac.mint(&contract_id, &1_000_000_000_000i128); // 100,000.0000000

    (
        env,
        client,
        admin,
        token_address,
        token_client,
        token_sac,
        contract_id,
    )
}

// TEST 1: HAPPY PATH - full proposal lifecycle
// Initialize -> whitelist org -> org creates proposal -> 3 students
// vote YES -> quorum met -> auto-executes -> USDC lands in org wallet.
#[test]
fn test_happy_path_proposal_to_disbursement() {
    let (env, client, admin, _token_addr, token_client, _sac, contract_addr) = setup_env();

    // Accredited org: BatStateU Mabini IT Society.
    let org = Address::generate(&env);
    client.whitelist_org(&admin, &org);

    // Org creates a proposal: 2,500 USDC.
    let campus = String::from_str(&env, "MABINI");
    let amount: i128 = 25_000_000_000;
    let desc = String::from_str(&env, "IT Week 2026 speaker fees");

    let proposal_id = client.create_proposal(&org, &campus, &amount, &desc);
    assert_eq!(proposal_id, 0u64);

    // Three students vote YES (meets QUORUM = 3 with majority).
    let student_a = Address::generate(&env);
    let student_b = Address::generate(&env);
    let student_c = Address::generate(&env);

    client.vote_proposal(&student_a, &proposal_id, &true);
    client.vote_proposal(&student_b, &proposal_id, &true);
    let final_status = client.vote_proposal(&student_c, &proposal_id, &true);

    // Proposal should now be Executed.
    assert_eq!(final_status, ProposalStatus::Executed);

    // Org wallet received 2,500 USDC.
    let org_balance = token_client.balance(&org);
    assert_eq!(org_balance, amount);

    // Treasury decreased by the same amount.
    let treasury_balance = token_client.balance(&contract_addr);
    assert_eq!(treasury_balance, 1_000_000_000_000i128 - amount);
}

// TEST 2: EDGE CASE - double voting is rejected
#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_double_vote_rejected() {
    let (env, client, admin, _token_addr, _token_client, _sac, _contract_addr) = setup_env();

    let org = Address::generate(&env);
    client.whitelist_org(&admin, &org);

    let campus = String::from_str(&env, "LOBO");
    let amount: i128 = 10_000_000_000;
    let desc = String::from_str(&env, "Lobo campus cleanup drive");

    let proposal_id = client.create_proposal(&org, &campus, &amount, &desc);

    let student = Address::generate(&env);

    // First vote: should succeed.
    client.vote_proposal(&student, &proposal_id, &true);

    // Second vote by the SAME student on the SAME proposal: must panic.
    client.vote_proposal(&student, &proposal_id, &true);
}

// TEST 3: STATE VERIFICATION after execution
#[test]
fn test_state_verification_after_execution() {
    let (env, client, admin, _token_addr, token_client, _sac, _contract_addr) = setup_env();

    let org = Address::generate(&env);
    client.whitelist_org(&admin, &org);

    let campus = String::from_str(&env, "BALAYAN");
    let amount: i128 = 15_000_000_000;
    let desc = String::from_str(&env, "Balayan org foundation day");

    let proposal_id = client.create_proposal(&org, &campus, &amount, &desc);

    // 3 votes to trigger execution.
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    client.vote_proposal(&s1, &proposal_id, &true);
    client.vote_proposal(&s2, &proposal_id, &false); // 1 NO
    client.vote_proposal(&s3, &proposal_id, &true); // 2 YES vs 1 NO -> majority YES

    // Verify proposal state via get_proposal().
    let prop = client.get_proposal(&proposal_id);

    assert_eq!(prop.campus, String::from_str(&env, "BALAYAN"));
    assert_eq!(prop.amount, amount);
    assert_eq!(prop.org_wallet, org);
    assert_eq!(prop.status, ProposalStatus::Executed);
    assert_eq!(prop.yes_votes, 2);
    assert_eq!(prop.no_votes, 1);

    // Verify treasury balance reflects the deduction.
    let treasury_bal = client.get_treasury_balance();
    assert_eq!(treasury_bal, 1_000_000_000_000i128 - amount);

    // Verify org wallet received the correct amount.
    assert_eq!(token_client.balance(&org), amount);
}
