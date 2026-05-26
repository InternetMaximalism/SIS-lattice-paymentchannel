use sha3::{Digest, Keccak256};

pub type Hash32 = [u8; 32];
pub type Address20 = [u8; 20];
pub type LatticeCommitment = [u64; sis_amount_stark::params::M];
pub type LatticeRandomness = [i64; sis_amount_stark::params::N];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelParams {
    pub channel_id: Hash32,
    pub asset_id: Hash32,
    pub participant_count: u16,
    pub participants: Vec<Participant>,
    pub challenge_period: u64,
    pub settlement_contract: Address20,
    pub total_balance_verifier: Address20,
    pub proof_system_id: Hash32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    pub index: u16,
    pub signing_key: Address20,
    pub withdraw_key: Address20,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantLeaf {
    pub channel_id: Hash32,
    pub state_version: u64,
    pub participant_index: u16,
    pub withdraw_key: Address20,
    pub balance_commitment: LatticeCommitment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffchainState {
    pub channel_id: Hash32,
    pub version: u64,
    pub prev_state_hash: Hash32,
    pub participant_root: Hash32,
    pub participants: Vec<ParticipantLeaf>,
    pub transition_meta_hash: Hash32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticipantSignature {
    pub participant_index: u16,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedState {
    pub state: OffchainState,
    pub signatures: Vec<ParticipantSignature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateProposalBundle {
    pub channel_id: Hash32,
    pub next_version: u64,
    pub prev_state_hash: Hash32,
    pub sender_index: u16,
    pub receiver_index: u16,
    pub amount_commitment: LatticeCommitment,
    pub amount_proof: Vec<u8>,
    pub next_participants: Vec<ParticipantLeaf>,
    pub sender_post_balance_proof: Vec<u8>,
    pub proof_format_version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiverWitnessShare {
    pub channel_id: Hash32,
    pub next_version: u64,
    pub sender_index: u16,
    pub receiver_index: u16,
    pub delta: u64,
    pub r_amount: LatticeRandomness,
}

pub fn empty_hash() -> Hash32 {
    hash_bytes(b"mpc:empty:v1")
}

pub fn hash_bytes(bytes: &[u8]) -> Hash32 {
    let digest = Keccak256::digest(bytes);
    digest.into()
}

pub fn leaf_hash(leaf: &ParticipantLeaf) -> Hash32 {
    let mut bytes = Vec::with_capacity(32 + 8 + 2 + 20 + (8 * sis_amount_stark::params::M) + 32);
    bytes.extend_from_slice(b"mpc:participant-leaf:v1");
    bytes.extend_from_slice(&leaf.channel_id);
    bytes.extend_from_slice(&leaf.state_version.to_be_bytes());
    bytes.extend_from_slice(&leaf.participant_index.to_be_bytes());
    bytes.extend_from_slice(&leaf.withdraw_key);
    for limb in leaf.balance_commitment {
        bytes.extend_from_slice(&limb.to_be_bytes());
    }
    hash_bytes(&bytes)
}

pub fn merkle_root(leaves: &[ParticipantLeaf]) -> Hash32 {
    if leaves.is_empty() {
        return empty_hash();
    }

    let mut level: Vec<Hash32> = leaves.iter().map(leaf_hash).collect();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut i = 0;
        while i < level.len() {
            let left = level[i];
            let right = if i + 1 < level.len() {
                level[i + 1]
            } else {
                level[i]
            };

            let mut bytes = Vec::with_capacity(32 + 32 + 20);
            bytes.extend_from_slice(b"mpc:merkle-node:v1");
            bytes.extend_from_slice(&left);
            bytes.extend_from_slice(&right);
            next.push(hash_bytes(&bytes));
            i += 2;
        }
        level = next;
    }

    level[0]
}

pub fn transition_meta_hash(bundle: &UpdateProposalBundle) -> Hash32 {
    let mut bytes = Vec::with_capacity(
        32 + 8
            + 32
            + 2
            + 2
            + (8 * sis_amount_stark::params::M)
            + bundle.amount_proof.len()
            + 4
            + bundle.sender_post_balance_proof.len(),
    );
    bytes.extend_from_slice(b"mpc:transition-meta:v1");
    bytes.extend_from_slice(&bundle.channel_id);
    bytes.extend_from_slice(&bundle.next_version.to_be_bytes());
    bytes.extend_from_slice(&bundle.prev_state_hash);
    bytes.extend_from_slice(&bundle.sender_index.to_be_bytes());
    bytes.extend_from_slice(&bundle.receiver_index.to_be_bytes());
    for limb in bundle.amount_commitment {
        bytes.extend_from_slice(&limb.to_be_bytes());
    }
    bytes.extend_from_slice(&bundle.amount_proof);
    bytes.extend_from_slice(&bundle.proof_format_version.to_be_bytes());
    bytes.extend_from_slice(&bundle.sender_post_balance_proof);
    hash_bytes(&bytes)
}

pub fn state_signing_hash(state: &OffchainState) -> Hash32 {
    let mut bytes = Vec::with_capacity(32 + 8 + 32 + 32 + 32 + 4 + (32 * state.participants.len()));
    bytes.extend_from_slice(b"mpc:offchain-state:v1");
    bytes.extend_from_slice(&state.channel_id);
    bytes.extend_from_slice(&state.version.to_be_bytes());
    bytes.extend_from_slice(&state.prev_state_hash);
    bytes.extend_from_slice(&state.participant_root);
    bytes.extend_from_slice(&state.transition_meta_hash);
    bytes.extend_from_slice(&(state.participants.len() as u32).to_be_bytes());
    for leaf in &state.participants {
        bytes.extend_from_slice(&leaf_hash(leaf));
    }
    hash_bytes(&bytes)
}

pub fn state_with_computed_root(
    channel_id: Hash32,
    version: u64,
    prev_state_hash: Hash32,
    participants: Vec<ParticipantLeaf>,
    transition_meta_hash: Hash32,
) -> OffchainState {
    let participant_root = merkle_root(&participants);
    OffchainState {
        channel_id,
        version,
        prev_state_hash,
        participant_root,
        participants,
        transition_meta_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sis_amount_stark::{compute_commitment, prove_amount, verify_amount};

    fn randomness_from_prefix(prefix: &[i64]) -> LatticeRandomness {
        let mut r = [0_i64; sis_amount_stark::params::N];
        for (idx, value) in prefix.iter().enumerate() {
            r[idx] = *value;
        }
        r
    }

    #[test]
    fn imported_sis_amount_stark_smoke_test() {
        let randomness = randomness_from_prefix(&[1, 0, -1, 2]);
        let (proof, public_inputs) = prove_amount(55, randomness).expect("proof should succeed");
        verify_amount(&proof, &public_inputs).expect("proof should verify");
        let expected = compute_commitment(55, &randomness);
        assert_eq!(public_inputs.c, expected.to_vec());
    }

    fn hash32(byte: u8) -> Hash32 {
        [byte; 32]
    }

    fn address20(byte: u8) -> Address20 {
        [byte; 20]
    }

    fn leaf(index: u16, commitment_seed: u64) -> ParticipantLeaf {
        ParticipantLeaf {
            channel_id: hash32(9),
            state_version: 3,
            participant_index: index,
            withdraw_key: address20(index as u8),
            balance_commitment: compute_commitment(
                commitment_seed,
                &randomness_from_prefix(&[
                    index as i64,
                    index as i64 + 1,
                    index as i64 + 2,
                    index as i64 + 3,
                ]),
            ),
        }
    }

    #[test]
    fn identical_state_hash_is_stable() {
        let participants = vec![leaf(0, 10), leaf(1, 20), leaf(2, 30)];
        let state =
            state_with_computed_root(hash32(1), 7, hash32(2), participants.clone(), hash32(3));
        let same_state = state_with_computed_root(hash32(1), 7, hash32(2), participants, hash32(3));

        assert_eq!(state.participant_root, same_state.participant_root);
        assert_eq!(state_signing_hash(&state), state_signing_hash(&same_state));
    }

    #[test]
    fn participant_order_changes_root_and_state_hash() {
        let ordered = vec![leaf(0, 10), leaf(1, 20), leaf(2, 30)];
        let reordered = vec![leaf(1, 20), leaf(0, 10), leaf(2, 30)];
        let state_a = state_with_computed_root(hash32(1), 7, hash32(2), ordered, hash32(3));
        let state_b = state_with_computed_root(hash32(1), 7, hash32(2), reordered, hash32(3));

        assert_ne!(state_a.participant_root, state_b.participant_root);
        assert_ne!(state_signing_hash(&state_a), state_signing_hash(&state_b));
    }

    #[test]
    fn mutating_a_leaf_changes_the_merkle_root() {
        let original = vec![leaf(0, 10), leaf(1, 20), leaf(2, 30)];
        let mut mutated = original.clone();
        mutated[1].balance_commitment[0] += 999;

        assert_ne!(merkle_root(&original), merkle_root(&mutated));
    }

    #[test]
    fn transition_meta_hash_changes_with_bundle_contents() {
        let base = UpdateProposalBundle {
            channel_id: hash32(1),
            next_version: 2,
            prev_state_hash: hash32(3),
            sender_index: 0,
            receiver_index: 1,
            amount_commitment: compute_commitment(11, &randomness_from_prefix(&[1, 2, 3, 4])),
            amount_proof: vec![9, 9],
            next_participants: vec![leaf(0, 10), leaf(1, 20)],
            sender_post_balance_proof: vec![1, 2, 3],
            proof_format_version: 1,
        };
        let mut changed = base.clone();
        changed.amount_commitment[0] += 1;

        assert_ne!(transition_meta_hash(&base), transition_meta_hash(&changed));
    }
}
