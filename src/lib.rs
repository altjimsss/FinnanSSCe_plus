#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, token, Address, Env,
    String,
};

// ERROR CODES
// Custom errors give clear feedback to callers and frontends.
// Each variant maps to a specific governance rule violation.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not been initialized yet.
    NotInitialized = 2,
    /// Caller is not the treasury admin (SSC Alangilan treasurer).
    NotAdmin = 3,
    /// Org wallet is not on the whitelist of accredited organizations.
    OrgNotWhitelisted = 4,
    /// Proposal with this ID does not exist.
    ProposalNotFound = 5,
    /// Student has already voted on this proposal - prevents double-voting
    /// so each contributor gets exactly one voice per proposal.
    AlreadyVoted = 6,
    /// Proposal has already been executed or rejected; no further action.
    ProposalNotPending = 7,
    /// Treasury does not hold enough USDC to cover the disbursement.
    InsufficientTreasury = 8,
}

// STORAGE KEYS
// We use an enum to strongly type every key stored in the contract's
// persistent storage. This prevents key-collision bugs.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Stores the Address of the treasury admin (SSC Alangilan).
    Admin,
    /// Stores the Address of the USDC token contract (SAC).
    TokenContract,
    /// Stores the next available proposal ID (auto-increment).
    NextProposalId,
    /// Stores a Proposal struct, keyed by its u64 ID.
    Proposal(u64),
    /// Tracks whether a (voter, proposal_id) pair has already voted.
    /// Prevents double-voting - critical for fair campus representation.
    Voted(Address, u64),
    /// Tracks whitelisted org addresses (true = accredited).
    Whitelist(Address),
}

// PROPOSAL STATUS
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ProposalStatus {
    Pending = 0,
    Executed = 1,
    Rejected = 2,
}

// PROPOSAL DATA MODEL
// Each funding proposal stores everything needed for transparency:
// which campus, which org, how much, what for, and the vote tally.
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    /// Org wallet that will receive USDC upon approval.
    pub org_wallet: Address,
    /// Campus tag: "MABINI", "LOBO", "BALAYAN", or "ALANGILAN".
    pub campus: String,
    /// Requested USDC amount in the token's smallest unit (7 decimals).
    pub amount: i128,
    /// Human-readable description.
    pub description: String,
    /// Current status: Pending, Executed, or Rejected.
    pub status: ProposalStatus,
    /// Number of YES votes received.
    pub yes_votes: u64,
    /// Number of NO votes received.
    pub no_votes: u64,
}

#[contractevent(topics = ["init"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitEvent {
    pub admin: Address,
}

#[contractevent(topics = ["wl_org"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WhitelistOrgEvent {
    pub org: Address,
}

#[contractevent(topics = ["prop_new"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalCreatedEvent {
    #[topic]
    pub proposal_id: u64,
    pub org_wallet: Address,
    pub campus: String,
    pub amount: i128,
}

#[contractevent(topics = ["vote"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteCastEvent {
    #[topic]
    pub proposal_id: u64,
    #[topic]
    pub voter: Address,
    pub vote_yes: bool,
}

#[contractevent(topics = ["disburse"])]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundsDisbursedEvent {
    #[topic]
    pub proposal_id: u64,
    pub org_wallet: Address,
    pub campus: String,
    pub amount: i128,
}

// GOVERNANCE CONSTANTS
/// Minimum total votes (YES + NO) before a proposal can finalize.
/// Set to 3 for easy demo; real deployment might be 50+ per campus.
const QUORUM: u64 = 3;

// CONTRACT DEFINITION
#[contract]
pub struct FinnanSSCe;

#[contractimpl]
impl FinnanSSCe {
    // Called once by the SSC Alangilan treasurer to bootstrap the contract.
    // Sets the admin address and the USDC token contract address.
    // The treasury balance is simply whatever USDC the admin transfers to
    // this contract's address.
    pub fn initialize(env: Env, admin: Address, token_contract: Address) -> Result<(), Error> {
        // Prevent re-initialization (immutable admin after setup).
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        // Require the admin to authorize this call.
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::TokenContract, &token_contract);
        env.storage().instance().set(&DataKey::NextProposalId, &0u64);

        // Emit initialization event for ledger traceability.
        InitEvent {
            admin: admin.clone(),
        }
        .publish(&env);

        Ok(())
    }

    // Admin-only. Adds an accredited student org to the whitelist.
    // Only whitelisted orgs can create proposals, preventing spam.
    pub fn whitelist_org(env: Env, admin: Address, org: Address) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();

        env.storage()
            .persistent()
            .set(&DataKey::Whitelist(org.clone()), &true);

        WhitelistOrgEvent { org }.publish(&env);

        Ok(())
    }

    // An accredited org wallet creates a funding proposal.
    pub fn create_proposal(
        env: Env,
        org_wallet: Address,
        campus: String,
        amount: i128,
        description: String,
    ) -> Result<u64, Error> {
        // Ensure contract is initialized.
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        // Only whitelisted (accredited) orgs can create proposals.
        let is_whitelisted: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Whitelist(org_wallet.clone()))
            .unwrap_or(false);

        if !is_whitelisted {
            return Err(Error::OrgNotWhitelisted);
        }

        // Org must authorize the creation (proves wallet ownership).
        org_wallet.require_auth();

        // Auto-increment proposal ID.
        let proposal_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::NextProposalId)
            .unwrap_or(0u64);

        let proposal = Proposal {
            id: proposal_id,
            org_wallet: org_wallet.clone(),
            campus: campus.clone(),
            amount,
            description: description.clone(),
            status: ProposalStatus::Pending,
            yes_votes: 0,
            no_votes: 0,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);
        env.storage()
            .instance()
            .set(&DataKey::NextProposalId, &(proposal_id + 1));

        // Event: ProposalCreated.
        ProposalCreatedEvent {
            proposal_id,
            org_wallet,
            campus,
            amount,
        }
        .publish(&env);

        Ok(proposal_id)
    }

    // A student wallet casts a YES (true) or NO (false) vote.
    // If quorum is reached and majority is YES, the proposal auto-executes.
    pub fn vote_proposal(
        env: Env,
        voter: Address,
        proposal_id: u64,
        vote_yes: bool,
    ) -> Result<ProposalStatus, Error> {
        // Voter must authorize (proves they own this wallet).
        voter.require_auth();

        // Load proposal or fail.
        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::ProposalNotFound)?;

        // Proposal must still be pending.
        if proposal.status != ProposalStatus::Pending {
            return Err(Error::ProposalNotPending);
        }

        // Check double-vote: one student, one voice per proposal.
        let vote_key = DataKey::Voted(voter.clone(), proposal_id);
        if env.storage().persistent().has(&vote_key) {
            return Err(Error::AlreadyVoted);
        }

        // Record vote.
        if vote_yes {
            proposal.yes_votes += 1;
        } else {
            proposal.no_votes += 1;
        }

        // Mark voter as having voted on this proposal.
        env.storage().persistent().set(&vote_key, &true);

        // Event: VoteCast.
        VoteCastEvent {
            proposal_id,
            voter: voter.clone(),
            vote_yes,
        }
        .publish(&env);

        // Check quorum and majority.
        let total_votes = proposal.yes_votes + proposal.no_votes;
        if total_votes >= QUORUM {
            if proposal.yes_votes > proposal.no_votes {
                // Auto-execute: transfer USDC to org wallet.
                Self::execute_internal(&env, &mut proposal)?;
            } else {
                proposal.status = ProposalStatus::Rejected;
            }
        }

        // Persist updated proposal.
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        Ok(proposal.status.clone())
    }

    // Admin-callable fallback execution.
    pub fn execute_proposal(env: Env, admin: Address, proposal_id: u64) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }
        admin.require_auth();

        let mut proposal: Proposal = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::ProposalNotFound)?;

        if proposal.status != ProposalStatus::Pending {
            return Err(Error::ProposalNotPending);
        }

        Self::execute_internal(&env, &mut proposal)?;

        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        Ok(())
    }

    // Shared disbursement logic.
    fn execute_internal(env: &Env, proposal: &mut Proposal) -> Result<(), Error> {
        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::TokenContract)
            .ok_or(Error::NotInitialized)?;

        let token_client = token::TokenClient::new(env, &token_address);

        // Check treasury (contract) has enough USDC.
        let contract_address = env.current_contract_address();
        let balance = token_client.balance(&contract_address);

        if balance < proposal.amount {
            return Err(Error::InsufficientTreasury);
        }

        // Transfer USDC from treasury (this contract) to org wallet.
        token_client.transfer(&contract_address, &proposal.org_wallet, &proposal.amount);

        proposal.status = ProposalStatus::Executed;

        // Event: FundsDisbursed.
        FundsDisbursedEvent {
            proposal_id: proposal.id,
            org_wallet: proposal.org_wallet.clone(),
            campus: proposal.campus.clone(),
            amount: proposal.amount,
        }
        .publish(env);

        Ok(())
    }

    // Returns full proposal details for the frontend dashboard.
    pub fn get_proposal(env: Env, proposal_id: u64) -> Result<Proposal, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(proposal_id))
            .ok_or(Error::ProposalNotFound)
    }

    // Returns the current USDC balance held by this contract.
    pub fn get_treasury_balance(env: Env) -> Result<i128, Error> {
        let token_address: Address = env
            .storage()
            .instance()
            .get(&DataKey::TokenContract)
            .ok_or(Error::NotInitialized)?;

        let token_client = token::TokenClient::new(&env, &token_address);
        let balance = token_client.balance(&env.current_contract_address());

        Ok(balance)
    }
}

#[cfg(test)]
mod test;
