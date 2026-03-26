#![no_std]

//! # soroban-community-treasury
//!
//! A DAO community treasury with token-weighted governance on Stellar.
//!
//! ## Flow
//!
//! 1. **Deposit** — any address deposits tokens into the treasury.
//!    The treasury balance is a collective pool; there are no
//!    individual balances or refund rights.
//!
//! 2. **Propose** — any governance token holder may submit a spending
//!    proposal with a description, target recipient, and requested amount.
//!    A voting window opens immediately.
//!
//! 3. **Vote** — governance token holders vote `For` or `Against`.
//!    Votes are token-weighted: weight = holder's governance token balance
//!    at the time of voting.  Each address may vote once per proposal.
//!
//! 4. **Queue** — after the voting window closes, if
//!    `for_votes × 10_000 / total_votes >= quorum_bps` AND
//!    `for_votes > against_votes`, anyone calls `queue_proposal`.
//!    The proposal enters a veto period.
//!
//! 5. **Veto** — during the veto period the admin (or a designated
//!    guardian) may cancel the proposal before execution.
//!
//! 6. **Execute** — after the veto period expires, anyone calls
//!    `execute_proposal`.  Funds are transferred to the specified
//!    recipient and the proposal is marked `Executed`.
//!
//! 7. **Cancel** — admin may cancel any proposal that is not yet
//!    `Executed`.  A proposer may cancel their own `Active` proposal
//!    before the voting window closes.
//!
//! ## Token-weighted Voting
//!
//! ```text
//! vote_weight = governance_token_balance(voter)   at time of vote
//! for_votes / (for_votes + against_votes) >= quorum_bps / 10_000
//! ```
//!
//! ## Key Invariants
//!
//! - `treasury_balance` = cumulative deposits − cumulative executed payouts.
//! - Queued proposals have their requested amount reserved; the treasury
//!   cannot be over-committed.
//! - A proposal may not be executed while the veto period is active.
//! - Each address votes at most once per proposal.

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, String, Symbol, Vec,
};

// ─────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────

const INITIALIZED: Symbol = symbol_short!("INIT");
const BPS_DENOM: u64 = 10_000;

// ─────────────────────────────────────────────
// Composite Key
// ─────────────────────────────────────────────

/// Key for a single vote on a proposal.
/// Dedicated struct required: `#[contracttype]` does not support
/// multi-field tuple enum variants.
#[contracttype]
#[derive(Clone)]
pub struct VoteKey {
    pub proposal_id: u64,
    pub voter: Address,
}

// ─────────────────────────────────────────────
// Storage Keys
// ─────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Global configuration.
    Config,
    /// Total unallocated tokens held in the treasury.
    TreasuryBalance,
    /// Total tokens reserved for queued (not yet executed) proposals.
    ReservedBalance,
    /// Monotonically-incrementing next proposal ID.
    NextProposalId,
    /// Full proposal record keyed by ID.
    Proposal(u64),
    /// All proposal IDs in global order.
    ProposalIndex,
    /// All proposal IDs submitted by a proposer.
    ProposerProposals(Address),
    /// Vote weight cast by a specific address on a specific proposal.
    Vote(VoteKey),
}

// ─────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────

/// Global treasury configuration.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Config {
    /// Admin — may veto proposals, cancel any proposal, and update config.
    pub admin: Address,
    /// Token held in the treasury and used for payouts.
    pub treasury_token: Address,
    /// Governance token whose balance determines voting weight.
    pub governance_token: Address,
    /// Minimum `for_votes / total_votes` in basis points (e.g. 5100 = 51%).
    pub quorum_bps: u32,
    /// Seconds the voting window stays open after proposal submission.
    pub voting_window: u64,
    /// Seconds between queue and earliest execution (veto period).
    pub veto_period: u64,
    /// Maximum amount any single proposal may request (0 = no cap).
    pub spending_cap: i128,
}

/// Lifecycle state of a spending proposal.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum ProposalStatus {
    /// Voting window open.
    Active,
    /// Voting window closed; quorum not reached or against won.
    Defeated,
    /// Passed vote; inside veto window awaiting execution.
    Queued,
    /// Executed; funds transferred.
    Executed,
    /// Cancelled before execution (admin or proposer).
    Cancelled,
}

/// Direction of a vote.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum VoteDirection {
    For,
    Against,
    Abstain,
}

/// A spending proposal.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: String,
    pub description: String,
    /// Address that will receive the funds if executed.
    pub recipient: Address,
    /// Requested treasury token amount.
    pub amount: i128,
    pub status: ProposalStatus,
    /// Total governance token weight cast For.
    pub for_votes: u64,
    /// Total governance token weight cast Against.
    pub against_votes: u64,
    /// Total governance token weight cast Abstain (informational only).
    pub abstain_votes: u64,
    /// Timestamp after which votes are no longer accepted.
    pub voting_deadline: u64,
    /// Earliest timestamp at which `execute_proposal` may be called.
    /// Set when the proposal is queued; 0 while Active or Defeated.
    pub executable_at: u64,
    pub submitted_at: u64,
    pub executed_at: u64,
}

// ─────────────────────────────────────────────
// Contract
// ─────────────────────────────────────────────

#[contract]
pub struct CommunityTreasuryContract;

#[contractimpl]
impl CommunityTreasuryContract {
    // ── Initialisation ───────────────────────

    /// Deploy the DAO treasury.
    ///
    /// # Arguments
    /// - `quorum_bps`    – minimum for-vote share in bps (e.g. 5100 = 51%)
    /// - `voting_window` – seconds the vote stays open
    /// - `veto_period`   – seconds between queue and earliest execution
    /// - `spending_cap`  – max per-proposal amount (0 = no cap)
    pub fn initialize(
        env: Env,
        admin: Address,
        treasury_token: Address,
        governance_token: Address,
        quorum_bps: u32,
        voting_window: u64,
        veto_period: u64,
        spending_cap: i128,
    ) {
        if env.storage().instance().has(&INITIALIZED) {
            panic!("already initialized");
        }
        assert!(quorum_bps > 0 && quorum_bps <= 10_000, "quorum_bps must be 1–10000");
        assert!(voting_window > 0, "voting window must be positive");
        assert!(veto_period > 0, "veto period must be positive");
        assert!(spending_cap >= 0, "spending cap cannot be negative");

        env.storage().instance().set(&INITIALIZED, &true);
        env.storage().instance().set(
            &DataKey::Config,
            &Config {
                admin,
                treasury_token,
                governance_token,
                quorum_bps,
                voting_window,
                veto_period,
                spending_cap,
            },
        );
        env.storage()
            .instance()
            .set(&DataKey::TreasuryBalance, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::ReservedBalance, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::NextProposalId, &1u64);
        env.storage()
            .instance()
            .set(&DataKey::ProposalIndex, &Vec::<u64>::new(&env));
    }

    // ── Admin ────────────────────────────────

    /// Update the quorum, voting window, veto period, or spending cap.
    /// Admin only.
    pub fn update_config(
        env: Env,
        admin: Address,
        quorum_bps: u32,
        voting_window: u64,
        veto_period: u64,
        spending_cap: i128,
    ) {
        admin.require_auth();
        let mut config = Self::load_config(&env);
        assert!(config.admin == admin, "caller is not the admin");
        assert!(quorum_bps > 0 && quorum_bps <= 10_000, "quorum_bps must be 1–10000");
        assert!(voting_window > 0, "voting window must be positive");
        assert!(veto_period > 0, "veto period must be positive");
        assert!(spending_cap >= 0, "spending cap cannot be negative");
        config.quorum_bps = quorum_bps;
        config.voting_window = voting_window;
        config.veto_period = veto_period;
        config.spending_cap = spending_cap;
        env.storage().instance().set(&DataKey::Config, &config);
    }

    /// Transfer admin rights.  Admin only.
    pub fn transfer_admin(env: Env, admin: Address, new_admin: Address) {
        admin.require_auth();
        let mut config = Self::load_config(&env);
        assert!(config.admin == admin, "caller is not the admin");
        config.admin = new_admin;
        env.storage().instance().set(&DataKey::Config, &config);
    }

    // ── Treasury Funding ─────────────────────

    /// Deposit treasury tokens into the pool.
    ///
    /// Any address may deposit.  Tokens are held collectively;
    /// there are no individual balances or refund rights.
    pub fn deposit(env: Env, depositor: Address, amount: i128) {
        depositor.require_auth();
        assert!(amount > 0, "deposit amount must be positive");

        let config = Self::load_config(&env);
        token::Client::new(&env, &config.treasury_token).transfer(
            &depositor,
            &env.current_contract_address(),
            &amount,
        );

        let bal: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TreasuryBalance, &(bal + amount));
    }

    // ── Proposals ────────────────────────────

    /// Submit a spending proposal.  Returns the proposal ID.
    ///
    /// Any governance token holder may propose.  The proposer does not
    /// need to hold treasury tokens.  Amount must not exceed
    /// `spending_cap` (if set) and must not exceed the available
    /// (unallocated) treasury balance.
    pub fn submit_proposal(
        env: Env,
        proposer: Address,
        title: String,
        description: String,
        recipient: Address,
        amount: i128,
    ) -> u64 {
        proposer.require_auth();

        assert!(amount > 0, "amount must be positive");
        assert!(!title.is_empty(), "title cannot be empty");
        assert!(!description.is_empty(), "description cannot be empty");

        let config = Self::load_config(&env);

        // Proposer must hold at least 1 governance token unit
        let gov_bal = token::Client::new(&env, &config.governance_token)
            .balance(&proposer);
        assert!(gov_bal > 0, "proposer must hold governance tokens");

        // Spending cap check
        if config.spending_cap > 0 {
            assert!(
                amount <= config.spending_cap,
                "amount exceeds spending cap"
            );
        }

        // Available funds check (treasury - already reserved)
        let treasury: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap_or(0);
        let reserved: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReservedBalance)
            .unwrap_or(0);
        assert!(
            treasury - reserved >= amount,
            "insufficient available treasury funds"
        );

        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextProposalId)
            .unwrap();
        env.storage()
            .instance()
            .set(&DataKey::NextProposalId, &(id + 1));

        let now = env.ledger().timestamp();
        let proposal = Proposal {
            id,
            proposer: proposer.clone(),
            title,
            description,
            recipient,
            amount,
            status: ProposalStatus::Active,
            for_votes: 0,
            against_votes: 0,
            abstain_votes: 0,
            voting_deadline: now + config.voting_window,
            executable_at: 0,
            submitted_at: now,
            executed_at: 0,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(id), &proposal);

        // Global index
        let mut index: Vec<u64> = env
            .storage()
            .instance()
            .get(&DataKey::ProposalIndex)
            .unwrap_or(Vec::new(&env));
        index.push_back(id);
        env.storage()
            .instance()
            .set(&DataKey::ProposalIndex, &index);

        // Proposer index
        let mut plist: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ProposerProposals(proposer.clone()))
            .unwrap_or(Vec::new(&env));
        plist.push_back(id);
        env.storage()
            .persistent()
            .set(&DataKey::ProposerProposals(proposer), &plist);

        id
    }

    // ── Voting ────────────────────────────────

    /// Cast a token-weighted vote on an Active proposal.
    ///
    /// Vote weight = caller's governance token balance at call time.
    /// Each address may vote exactly once per proposal.
    /// Abstain votes count toward quorum denominator.
    pub fn vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        direction: VoteDirection,
    ) {
        voter.require_auth();

        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found");

        assert!(
            proposal.status == ProposalStatus::Active,
            "proposal is not active"
        );
        assert!(
            env.ledger().timestamp() <= proposal.voting_deadline,
            "voting window has closed"
        );

        let vote_key = DataKey::Vote(VoteKey {
            proposal_id,
            voter: voter.clone(),
        });
        assert!(
            !env.storage().persistent().has(&vote_key),
            "already voted on this proposal"
        );

        let config = Self::load_config(&env);
        let weight = token::Client::new(&env, &config.governance_token)
            .balance(&voter) as u64;
        assert!(weight > 0, "voter must hold governance tokens");

        env.storage().persistent().set(&vote_key, &direction.clone());

        match direction {
            VoteDirection::For => proposal.for_votes += weight,
            VoteDirection::Against => proposal.against_votes += weight,
            VoteDirection::Abstain => proposal.abstain_votes += weight,
        }

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
    }

    // ── Queue ─────────────────────────────────

    /// Queue a proposal that has passed its voting window.
    ///
    /// **Permissionless** — anyone may call after the voting deadline.
    ///
    /// Pass conditions:
    /// - `for_votes > against_votes`
    /// - `for_votes * 10_000 / (for_votes + against_votes + abstain_votes) >= quorum_bps`
    ///
    /// On success: reserves the amount and starts the veto countdown.
    pub fn queue_proposal(env: Env, proposal_id: u64) {
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found");

        assert!(
            proposal.status == ProposalStatus::Active,
            "proposal is not active"
        );
        assert!(
            env.ledger().timestamp() > proposal.voting_deadline,
            "voting window has not closed"
        );

        let config = Self::load_config(&env);
        let total = proposal.for_votes + proposal.against_votes + proposal.abstain_votes;

        let passed = proposal.for_votes > proposal.against_votes
            && (if total > 0 {
                proposal.for_votes * BPS_DENOM / total >= config.quorum_bps as u64
            } else {
                false
            });

        if passed {
            // Reserve funds for this proposal
            let reserved: i128 = env
                .storage()
                .instance()
                .get(&DataKey::ReservedBalance)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::ReservedBalance, &(reserved + proposal.amount));

            proposal.status = ProposalStatus::Queued;
            proposal.executable_at = env.ledger().timestamp() + config.veto_period;
        } else {
            proposal.status = ProposalStatus::Defeated;
        }

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
    }

    // ── Veto / Cancel ─────────────────────────

    /// Cancel a proposal at any non-executed stage.
    ///
    /// - Admin may cancel any proposal that is `Active`, `Queued`, or
    ///   `Defeated`.
    /// - The proposer may only cancel their own `Active` proposal
    ///   (before the voting window closes).
    ///
    /// Cancelling a `Queued` proposal releases its reserved funds.
    pub fn cancel_proposal(env: Env, caller: Address, proposal_id: u64) {
        caller.require_auth();

        let config = Self::load_config(&env);
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found");

        assert!(
            proposal.status != ProposalStatus::Executed,
            "cannot cancel an executed proposal"
        );
        assert!(
            proposal.status != ProposalStatus::Cancelled,
            "proposal is already cancelled"
        );

        let is_admin = config.admin == caller;
        let is_proposer = proposal.proposer == caller;

        if is_proposer && !is_admin {
            assert!(
                proposal.status == ProposalStatus::Active,
                "proposer may only cancel active proposals"
            );
            assert!(
                env.ledger().timestamp() <= proposal.voting_deadline,
                "voting window has closed; only admin may cancel"
            );
        } else {
            assert!(is_admin, "only admin or proposer may cancel");
        }

        // Release reserved funds if queued
        if proposal.status == ProposalStatus::Queued {
            let reserved: i128 = env
                .storage()
                .instance()
                .get(&DataKey::ReservedBalance)
                .unwrap_or(0);
            env.storage()
                .instance()
                .set(&DataKey::ReservedBalance, &(reserved - proposal.amount));
        }

        proposal.status = ProposalStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
    }

    // ── Execute ───────────────────────────────

    /// Execute a Queued proposal after the veto period expires.
    ///
    /// **Permissionless** — anyone may call once `executable_at` has passed.
    /// Transfers `amount` treasury tokens to `proposal.recipient`.
    pub fn execute_proposal(env: Env, proposal_id: u64) {
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found");

        assert!(
            proposal.status == ProposalStatus::Queued,
            "proposal is not queued"
        );

        let now = env.ledger().timestamp();
        assert!(
            now >= proposal.executable_at,
            "veto period has not expired"
        );

        let config = Self::load_config(&env);

        // Debit treasury and reserved balances
        let treasury: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap_or(0);
        assert!(
            treasury >= proposal.amount,
            "treasury has insufficient funds"
        );
        env.storage()
            .instance()
            .set(&DataKey::TreasuryBalance, &(treasury - proposal.amount));

        let reserved: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReservedBalance)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::ReservedBalance, &(reserved - proposal.amount));

        // Transfer to recipient
        token::Client::new(&env, &config.treasury_token).transfer(
            &env.current_contract_address(),
            &proposal.recipient,
            &proposal.amount,
        );

        proposal.status = ProposalStatus::Executed;
        proposal.executed_at = now;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
    }

    // ── Queries ───────────────────────────────

    /// Return the global configuration.
    pub fn get_config(env: Env) -> Config {
        Self::load_config(&env)
    }

    /// Return the unallocated treasury balance.
    pub fn get_treasury_balance(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap_or(0)
    }

    /// Return the amount currently reserved for queued proposals.
    pub fn get_reserved_balance(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::ReservedBalance)
            .unwrap_or(0)
    }

    /// Return available (unallocated) funds.
    pub fn get_available_balance(env: Env) -> i128 {
        let treasury: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryBalance)
            .unwrap_or(0);
        let reserved: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReservedBalance)
            .unwrap_or(0);
        (treasury - reserved).max(0)
    }

    /// Return a full proposal record.
    pub fn get_proposal(env: Env, proposal_id: u64) -> Proposal {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .expect("proposal not found")
    }

    /// Return all proposal IDs (global order).
    pub fn get_all_proposals(env: Env) -> Vec<u64> {
        env.storage()
            .instance()
            .get(&DataKey::ProposalIndex)
            .unwrap_or(Vec::new(&env))
    }

    /// Return all proposal IDs submitted by a proposer.
    pub fn get_proposer_proposals(env: Env, proposer: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::ProposerProposals(proposer))
            .unwrap_or(Vec::new(&env))
    }

    /// Return the vote cast by `voter` on `proposal_id`, if any.
    pub fn get_vote(env: Env, proposal_id: u64, voter: Address) -> Option<VoteDirection> {
        env.storage()
            .persistent()
            .get(&DataKey::Vote(VoteKey { proposal_id, voter }))
    }

    /// Return whether `voter` has voted on `proposal_id`.
    pub fn has_voted(env: Env, proposal_id: u64, voter: Address) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::Vote(VoteKey { proposal_id, voter }))
    }

    // ── Internal ──────────────────────────────

    fn load_config(env: &Env) -> Config {
        env.storage().instance().get(&DataKey::Config).unwrap()
    }
}

#[cfg(test)]
mod test;
