#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Address as _, testutils::Ledger as _, Env, Vec};
use time_lock::{TimeLock, TimeLockClient};

fn setup(env: &Env) -> (TreasuryClient<'_>, Address, Vec<Address>) {
    env.mock_all_auths();
    let admin = Address::generate(env);

    let timelock_id = env.register(TimeLock, ());
    let timelock = TimeLockClient::new(env, &timelock_id);
    timelock.initialize(&admin, &3600, &86400, &1);

    let contract_id = env.register(Treasury, ());
    let client = TreasuryClient::new(env, &contract_id);

    let mut signers = Vec::new(env);
    signers.push_back(admin.clone());
    let signer2 = Address::generate(env);
    signers.push_back(signer2);

    client.initialize(&admin, &signers, &2, &3600, &timelock_id);
    timelock.add_governor(&admin, &contract_id);
    (client, admin, signers)
}

#[test]
fn pause_round_trip() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env);

    assert!(!client.is_paused());
    client.set_paused(&_admin, &true);
    assert!(client.is_paused());
    client.set_paused(&_admin, &false);
    assert!(!client.is_paused());
}

#[test]
#[should_panic(expected = "caller is not admin")]
fn pause_by_non_admin_fails() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env);

    let intruder = Address::generate(&env);
    client.set_paused(&intruder, &true);
}

#[test]
#[should_panic(expected = "contract is paused")]
fn deposit_blocked_while_paused() {
    let env = Env::default();
    let (client, _admin, _signers) = setup(&env);

    client.set_paused(&_admin, &true);
    let depositor = Address::generate(&env);
    client.deposit(&depositor, &100);
}

#[test]
#[should_panic(expected = "contract is paused")]
fn spending_proposal_blocked_while_paused() {
    let env = Env::default();
    let (client, admin, signers) = setup(&env);

    client.set_paused(&admin, &true);
    let recipient = Address::generate(&env);
    client.create_spending_proposal(
        &signers.get(1).unwrap(),
        &recipient,
        &100,
        &Symbol::new(&env, "ops"),
        &String::from_str(&env, "paused treasury"),
    );
}

#[test]
fn reads_work_while_paused() {
    let env = Env::default();
    let (client, admin, signers) = setup(&env);

    let depositor = Address::generate(&env);
    client.deposit(&depositor, &500);

    client.set_paused(&admin, &true);

    // Read entry points must stay available during an emergency stop.
    assert!(client.is_paused());
    assert_eq!(client.get_balance(), 500);
    assert_eq!(client.get_signers().len(), signers.len());
    assert_eq!(client.get_threshold(), 2);
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_dashboard().balance, 500);
}

#[test]
fn unpause_restores_mutations() {
    let env = Env::default();
    let (client, admin, _signers) = setup(&env);

    client.set_paused(&admin, &true);
    let depositor = Address::generate(&env);

    // Unpause restores normal operation.
    client.set_paused(&admin, &false);
    client.deposit(&depositor, &250);
    assert_eq!(client.get_balance(), 250);
}

#[test]
fn timelock_integration_duration_change_preserves_execute_after() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);

    let timelock_id = env.register(TimeLock, ());
    let timelock = TimeLockClient::new(&env, &timelock_id);
    timelock.initialize(&admin, &3600, &86400, &1);

    let treasury_id = env.register(Treasury, ());
    let client = TreasuryClient::new(&env, &treasury_id);

    let mut signers = Vec::new(&env);
    signers.push_back(admin.clone());
    let signer2 = Address::generate(&env);
    signers.push_back(signer2.clone());

    client.initialize(&admin, &signers, &2, &3600, &timelock_id);
    timelock.add_governor(&admin, &treasury_id);

    let depositor = Address::generate(&env);
    client.deposit(&depositor, &10000);

    let category = Symbol::new(&env, "ops");
    let alloc_id = client.propose_budget_allocation(&admin, &category, &10000);
    client.vote_budget_allocation(&signer2, &alloc_id, &true);
    client.finalize_budget_allocation(&admin, &alloc_id);

    let recipient = Address::generate(&env);
    let proposal_id = client.create_spending_proposal(
        &admin,
        &recipient,
        &1000,
        &category,
        &String::from_str(&env, "high-value spend"),
    );
    let original = client.get_spending_proposal(&proposal_id).unwrap();
    assert_eq!(original.execute_after, 3600);
    client.approve_proposal(&signer2, &proposal_id);

    let duration_id = client.propose_time_lock_update(&admin, &0);
    client.vote_time_lock_update(&signer2, &duration_id, &true);
    client.finalize_time_lock_update(&admin, &duration_id);
    assert_eq!(client.get_dashboard().time_lock_duration, 0);

    let preserved = client.get_spending_proposal(&proposal_id).unwrap();
    assert_eq!(preserved.execute_after, 3600);

    env.ledger().with_mut(|l| l.timestamp = 100);
    assert!(client.try_execute_proposal(&admin, &proposal_id).is_err());

    env.ledger().with_mut(|l| l.timestamp = 3600);
    client.execute_proposal(&admin, &proposal_id);
    let executed = client.get_spending_proposal(&proposal_id).unwrap();
    assert!(executed.executed);
    assert_eq!(client.get_balance(), 9000);
}
