use channel_types::{
    LatticeCommitment, OffchainState, ParticipantLeaf, ReceiverWitnessShare, UpdateProposalBundle,
    state_signing_hash, state_with_computed_root, transition_meta_hash,
};
use proof_adapter::{
    BalanceProof, DualBetaPreparedSystems, HiddenAmountWitness, ProofAdapterError,
    TransferAmountProof, add_commitments, apply_amount_to_receiver_balance,
    apply_amount_to_sender_balance, compute_lattice_commitment, prove_balance_opening,
    prove_transfer_amount, sub_commitments, verify_balance_proof, verify_transfer_amount_proof,
    verify_witness_matches_commitment,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChannelStateError {
    #[error(transparent)]
    Proof(#[from] ProofAdapterError),
    #[error("invalid participant indices")]
    InvalidParticipantIndices,
    #[error("participant not found: {0}")]
    ParticipantNotFound(u16),
    #[error("channel id mismatch")]
    ChannelIdMismatch,
    #[error("next version mismatch")]
    NextVersionMismatch,
    #[error("previous state hash mismatch")]
    PrevStateHashMismatch,
    #[error("participant count mismatch")]
    ParticipantCountMismatch,
    #[error("participant metadata changed unexpectedly for index {0}")]
    UnexpectedParticipantMetadata(u16),
    #[error("unchanged participant commitment changed for index {0}")]
    UnexpectedUnchangedCommitment(u16),
    #[error("amount proof commitment mismatch")]
    AmountProofCommitmentMismatch,
    #[error("sender commitment mismatch")]
    SenderCommitmentMismatch,
    #[error("receiver commitment mismatch")]
    ReceiverCommitmentMismatch,
    #[error("sender proof commitment mismatch")]
    SenderProofCommitmentMismatch,
    #[error("receiver witness share mismatch")]
    ReceiverWitnessShareMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferUpdateArtifacts {
    pub bundle: UpdateProposalBundle,
    pub next_state: OffchainState,
    pub receiver_share: ReceiverWitnessShare,
    pub sender_after_witness: HiddenAmountWitness,
}

fn participant_position(
    participants: &[ParticipantLeaf],
    participant_index: u16,
) -> Result<usize, ChannelStateError> {
    participants
        .iter()
        .position(|leaf| leaf.participant_index == participant_index)
        .ok_or(ChannelStateError::ParticipantNotFound(participant_index))
}

fn build_next_participants(
    prev_state: &OffchainState,
    sender_index: u16,
    receiver_index: u16,
    amount_commitment: &LatticeCommitment,
) -> Result<Vec<ParticipantLeaf>, ChannelStateError> {
    if sender_index == receiver_index {
        return Err(ChannelStateError::InvalidParticipantIndices);
    }

    let sender_pos = participant_position(&prev_state.participants, sender_index)?;
    let receiver_pos = participant_position(&prev_state.participants, receiver_index)?;
    let next_version = prev_state.version + 1;

    let mut next_participants = Vec::with_capacity(prev_state.participants.len());
    for (position, prev_leaf) in prev_state.participants.iter().enumerate() {
        let mut next_leaf = prev_leaf.clone();
        next_leaf.state_version = next_version;
        next_leaf.balance_commitment = if position == sender_pos {
            sub_commitments(&prev_leaf.balance_commitment, amount_commitment)
        } else if position == receiver_pos {
            add_commitments(&prev_leaf.balance_commitment, amount_commitment)
        } else {
            prev_leaf.balance_commitment
        };
        next_participants.push(next_leaf);
    }

    Ok(next_participants)
}

pub fn propose_transfer_update(
    systems: &DualBetaPreparedSystems,
    prev_state: &OffchainState,
    sender_index: u16,
    receiver_index: u16,
    sender_before_witness: &HiddenAmountWitness,
    amount_witness: &HiddenAmountWitness,
) -> Result<TransferUpdateArtifacts, ChannelStateError> {
    let sender_pos = participant_position(&prev_state.participants, sender_index)?;
    let sender_before_commitment = prev_state.participants[sender_pos].balance_commitment;
    verify_witness_matches_commitment(sender_before_witness, &sender_before_commitment)?;

    let sender_after_witness = apply_amount_to_sender_balance(
        sender_before_witness,
        amount_witness,
        systems.bounds().balance_randomness_beta,
    )?;
    let amount_commitment =
        compute_lattice_commitment(amount_witness.amount, &amount_witness.randomness);
    let next_participants =
        build_next_participants(prev_state, sender_index, receiver_index, &amount_commitment)?;
    let amount_proof = prove_transfer_amount(systems, amount_witness)?;
    let sender_post_balance_proof = prove_balance_opening(systems, &sender_after_witness)?;

    let bundle = UpdateProposalBundle {
        channel_id: prev_state.channel_id,
        next_version: prev_state.version + 1,
        prev_state_hash: state_signing_hash(prev_state),
        sender_index,
        receiver_index,
        amount_commitment,
        amount_proof: amount_proof.envelope_bytes,
        next_participants,
        sender_post_balance_proof: sender_post_balance_proof.envelope_bytes,
        proof_format_version: 2,
    };

    let next_state = state_with_computed_root(
        prev_state.channel_id,
        bundle.next_version,
        bundle.prev_state_hash,
        bundle.next_participants.clone(),
        transition_meta_hash(&bundle),
    );

    let receiver_share = ReceiverWitnessShare {
        channel_id: prev_state.channel_id,
        next_version: prev_state.version + 1,
        sender_index,
        receiver_index,
        delta: amount_witness.amount,
        r_amount: amount_witness.randomness,
    };

    Ok(TransferUpdateArtifacts {
        bundle,
        next_state,
        receiver_share,
        sender_after_witness,
    })
}

pub fn verify_update_bundle(
    systems: &DualBetaPreparedSystems,
    prev_state: &OffchainState,
    bundle: &UpdateProposalBundle,
) -> Result<OffchainState, ChannelStateError> {
    if bundle.channel_id != prev_state.channel_id {
        return Err(ChannelStateError::ChannelIdMismatch);
    }
    if bundle.next_version != prev_state.version + 1 {
        return Err(ChannelStateError::NextVersionMismatch);
    }
    if bundle.prev_state_hash != state_signing_hash(prev_state) {
        return Err(ChannelStateError::PrevStateHashMismatch);
    }
    if bundle.next_participants.len() != prev_state.participants.len() {
        return Err(ChannelStateError::ParticipantCountMismatch);
    }

    let sender_pos = participant_position(&bundle.next_participants, bundle.sender_index)?;
    let receiver_pos = participant_position(&bundle.next_participants, bundle.receiver_index)?;
    if sender_pos == receiver_pos {
        return Err(ChannelStateError::InvalidParticipantIndices);
    }

    let amount_proof = TransferAmountProof {
        envelope_bytes: bundle.amount_proof.clone(),
    };
    let verified_amount = verify_transfer_amount_proof(systems, &amount_proof)?;
    if verified_amount.commitment != bundle.amount_commitment {
        return Err(ChannelStateError::AmountProofCommitmentMismatch);
    }

    for (prev_leaf, next_leaf) in prev_state
        .participants
        .iter()
        .zip(&bundle.next_participants)
    {
        if next_leaf.channel_id != prev_state.channel_id
            || next_leaf.state_version != bundle.next_version
            || next_leaf.participant_index != prev_leaf.participant_index
            || next_leaf.withdraw_key != prev_leaf.withdraw_key
        {
            return Err(ChannelStateError::UnexpectedParticipantMetadata(
                prev_leaf.participant_index,
            ));
        }

        let expected_commitment = if prev_leaf.participant_index == bundle.sender_index {
            sub_commitments(&prev_leaf.balance_commitment, &bundle.amount_commitment)
        } else if prev_leaf.participant_index == bundle.receiver_index {
            add_commitments(&prev_leaf.balance_commitment, &bundle.amount_commitment)
        } else {
            prev_leaf.balance_commitment
        };

        if next_leaf.balance_commitment != expected_commitment {
            if prev_leaf.participant_index == bundle.sender_index {
                return Err(ChannelStateError::SenderCommitmentMismatch);
            }
            if prev_leaf.participant_index == bundle.receiver_index {
                return Err(ChannelStateError::ReceiverCommitmentMismatch);
            }
            return Err(ChannelStateError::UnexpectedUnchangedCommitment(
                prev_leaf.participant_index,
            ));
        }
    }

    let sender_proof = BalanceProof {
        envelope_bytes: bundle.sender_post_balance_proof.clone(),
    };
    let verified_sender = verify_balance_proof(systems, &sender_proof)?;
    if verified_sender.commitment != bundle.next_participants[sender_pos].balance_commitment {
        return Err(ChannelStateError::SenderProofCommitmentMismatch);
    }

    Ok(state_with_computed_root(
        prev_state.channel_id,
        bundle.next_version,
        bundle.prev_state_hash,
        bundle.next_participants.clone(),
        transition_meta_hash(bundle),
    ))
}

pub fn verify_receiver_witness_share(
    systems: &DualBetaPreparedSystems,
    prev_state: &OffchainState,
    bundle: &UpdateProposalBundle,
    receiver_before_witness: &HiddenAmountWitness,
    share: &ReceiverWitnessShare,
) -> Result<HiddenAmountWitness, ChannelStateError> {
    if share.channel_id != bundle.channel_id
        || share.next_version != bundle.next_version
        || share.sender_index != bundle.sender_index
        || share.receiver_index != bundle.receiver_index
    {
        return Err(ChannelStateError::ReceiverWitnessShareMismatch);
    }

    let amount_witness = HiddenAmountWitness {
        amount: share.delta,
        randomness: share.r_amount,
    };
    let share_commitment =
        compute_lattice_commitment(amount_witness.amount, &amount_witness.randomness);
    if share_commitment != bundle.amount_commitment {
        return Err(ChannelStateError::ReceiverWitnessShareMismatch);
    }

    let amount_proof = TransferAmountProof {
        envelope_bytes: bundle.amount_proof.clone(),
    };
    let verified_amount = verify_transfer_amount_proof(systems, &amount_proof)?;
    if verified_amount.commitment != share_commitment {
        return Err(ChannelStateError::ReceiverWitnessShareMismatch);
    }

    let receiver_pos = participant_position(&prev_state.participants, share.receiver_index)?;
    verify_witness_matches_commitment(
        receiver_before_witness,
        &prev_state.participants[receiver_pos].balance_commitment,
    )?;
    let receiver_after = apply_amount_to_receiver_balance(
        receiver_before_witness,
        &amount_witness,
        systems.bounds().balance_randomness_beta,
    )?;

    if bundle.next_participants[receiver_pos].balance_commitment
        != compute_lattice_commitment(receiver_after.amount, &receiver_after.randomness)
    {
        return Err(ChannelStateError::ReceiverWitnessShareMismatch);
    }

    Ok(receiver_after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use channel_types::Hash32;
    use proof_adapter::{DualBetaBounds, prepare_dual_beta_systems};

    fn systems() -> DualBetaPreparedSystems {
        prepare_dual_beta_systems(DualBetaBounds::new(31, 200_000).expect("valid bounds"))
            .expect("systems should prepare")
    }

    fn randomness_from_prefix(prefix: &[i64]) -> channel_types::LatticeRandomness {
        let mut r: channel_types::LatticeRandomness = core::array::from_fn(|_| 0_i64);
        for (idx, value) in prefix.iter().enumerate() {
            r[idx] = *value;
        }
        r
    }

    #[test]
    fn propose_and_verify_three_party_transfer() {
        let systems = systems();
        let channel_id = [7_u8; 32];
        let sender_before = HiddenAmountWitness {
            amount: 500,
            randomness: randomness_from_prefix(&[10, 10, 10, 10]),
        };
        let receiver_before = HiddenAmountWitness {
            amount: 100,
            randomness: randomness_from_prefix(&[2, 2, 2, 2]),
        };
        let prev_state = state_with_computed_root(
            channel_id,
            0,
            [0_u8; 32],
            vec![
                leaf(
                    channel_id,
                    0,
                    0,
                    compute_lattice_commitment(500, &sender_before.randomness),
                ),
                leaf(
                    channel_id,
                    0,
                    1,
                    compute_lattice_commitment(100, &receiver_before.randomness),
                ),
                leaf(
                    channel_id,
                    0,
                    2,
                    compute_lattice_commitment(75, &randomness_from_prefix(&[0, 0, 0, 0])),
                ),
            ],
            [1_u8; 32],
        );
        let amount_witness = HiddenAmountWitness {
            amount: 25,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };

        let artifacts =
            propose_transfer_update(&systems, &prev_state, 0, 1, &sender_before, &amount_witness)
                .expect("proposal should be built");
        let verified_state = verify_update_bundle(&systems, &prev_state, &artifacts.bundle)
            .expect("bundle should verify");

        assert_eq!(verified_state, artifacts.next_state);

        let receiver_after = verify_receiver_witness_share(
            &systems,
            &prev_state,
            &artifacts.bundle,
            &receiver_before,
            &artifacts.receiver_share,
        )
        .expect("receiver share should verify");
        assert_eq!(
            compute_lattice_commitment(receiver_after.amount, &receiver_after.randomness),
            artifacts.bundle.next_participants[1].balance_commitment
        );
    }

    #[test]
    fn tampered_receiver_share_is_rejected() {
        let systems = systems();
        let channel_id = [7_u8; 32];
        let prev_state = state_with_computed_root(
            channel_id,
            0,
            [0_u8; 32],
            vec![
                leaf(
                    channel_id,
                    0,
                    0,
                    compute_lattice_commitment(500, &randomness_from_prefix(&[10, 10, 10, 10])),
                ),
                leaf(
                    channel_id,
                    0,
                    1,
                    compute_lattice_commitment(100, &randomness_from_prefix(&[2, 2, 2, 2])),
                ),
            ],
            [1_u8; 32],
        );
        let sender_before = HiddenAmountWitness {
            amount: 500,
            randomness: randomness_from_prefix(&[10, 10, 10, 10]),
        };
        let receiver_before = HiddenAmountWitness {
            amount: 100,
            randomness: randomness_from_prefix(&[2, 2, 2, 2]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 25,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };

        let artifacts =
            propose_transfer_update(&systems, &prev_state, 0, 1, &sender_before, &amount_witness)
                .expect("proposal should be built");
        let mut tampered_share = artifacts.receiver_share.clone();
        tampered_share.delta += 1;

        assert!(
            verify_receiver_witness_share(
                &systems,
                &prev_state,
                &artifacts.bundle,
                &receiver_before,
                &tampered_share,
            )
            .is_err()
        );
    }

    #[test]
    fn tampered_amount_proof_is_rejected() {
        let systems = systems();
        let channel_id = [7_u8; 32];
        let prev_state = state_with_computed_root(
            channel_id,
            0,
            [0_u8; 32],
            vec![
                leaf(
                    channel_id,
                    0,
                    0,
                    compute_lattice_commitment(500, &randomness_from_prefix(&[10, 10, 10, 10])),
                ),
                leaf(
                    channel_id,
                    0,
                    1,
                    compute_lattice_commitment(100, &randomness_from_prefix(&[2, 2, 2, 2])),
                ),
            ],
            [1_u8; 32],
        );
        let sender_before = HiddenAmountWitness {
            amount: 500,
            randomness: randomness_from_prefix(&[10, 10, 10, 10]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 25,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };
        let mut artifacts =
            propose_transfer_update(&systems, &prev_state, 0, 1, &sender_before, &amount_witness)
                .expect("proposal should be built");
        artifacts.bundle.amount_proof[0] ^= 0x01;

        assert!(verify_update_bundle(&systems, &prev_state, &artifacts.bundle).is_err());
    }

    #[test]
    fn tampered_unchanged_participant_is_rejected() {
        let systems = systems();
        let channel_id = [7_u8; 32];
        let prev_state = state_with_computed_root(
            channel_id,
            0,
            [0_u8; 32],
            vec![
                leaf(
                    channel_id,
                    0,
                    0,
                    compute_lattice_commitment(500, &randomness_from_prefix(&[10, 10, 10, 10])),
                ),
                leaf(
                    channel_id,
                    0,
                    1,
                    compute_lattice_commitment(100, &randomness_from_prefix(&[2, 2, 2, 2])),
                ),
                leaf(
                    channel_id,
                    0,
                    2,
                    compute_lattice_commitment(75, &randomness_from_prefix(&[0, 0, 0, 0])),
                ),
            ],
            [1_u8; 32],
        );
        let sender_before = HiddenAmountWitness {
            amount: 500,
            randomness: randomness_from_prefix(&[10, 10, 10, 10]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 25,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };
        let mut artifacts =
            propose_transfer_update(&systems, &prev_state, 0, 1, &sender_before, &amount_witness)
                .expect("proposal should be built");
        artifacts.bundle.next_participants[2].balance_commitment[0] += 1;

        assert!(verify_update_bundle(&systems, &prev_state, &artifacts.bundle).is_err());
    }

    #[test]
    fn sender_proof_must_match_sender_next_commitment() {
        let systems = systems();
        let channel_id = [7_u8; 32];
        let prev_state = state_with_computed_root(
            channel_id,
            0,
            [0_u8; 32],
            vec![
                leaf(
                    channel_id,
                    0,
                    0,
                    compute_lattice_commitment(500, &randomness_from_prefix(&[10, 10, 10, 10])),
                ),
                leaf(
                    channel_id,
                    0,
                    1,
                    compute_lattice_commitment(100, &randomness_from_prefix(&[2, 2, 2, 2])),
                ),
            ],
            [1_u8; 32],
        );
        let sender_before = HiddenAmountWitness {
            amount: 500,
            randomness: randomness_from_prefix(&[10, 10, 10, 10]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 25,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };
        let mut artifacts =
            propose_transfer_update(&systems, &prev_state, 0, 1, &sender_before, &amount_witness)
                .expect("proposal should be built");
        artifacts.bundle.next_participants[0].balance_commitment[0] += 1;

        assert!(matches!(
            verify_update_bundle(&systems, &prev_state, &artifacts.bundle),
            Err(ChannelStateError::SenderCommitmentMismatch)
                | Err(ChannelStateError::SenderProofCommitmentMismatch)
        ));
    }

    fn leaf(
        channel_id: Hash32,
        state_version: u64,
        participant_index: u16,
        balance_commitment: channel_types::LatticeCommitment,
    ) -> ParticipantLeaf {
        ParticipantLeaf {
            channel_id,
            state_version,
            participant_index,
            withdraw_key: [participant_index as u8; 20],
            balance_commitment,
        }
    }
}
