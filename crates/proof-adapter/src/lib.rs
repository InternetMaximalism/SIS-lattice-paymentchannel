use channel_types::{LatticeCommitment, LatticeRandomness};
use sis_amount_stark::{
    PreparedSystem, ProofSystemOptions, PublicInputs, compute_commitment, deserialize_envelope,
    prepare_system, prove_amount_prepared, serialize_envelope, verify_amount_prepared,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProofAdapterError {
    #[error(transparent)]
    Upstream(#[from] sis_amount_stark::Error),
    #[error("amount underflow")]
    AmountUnderflow,
    #[error("amount overflow")]
    AmountOverflow,
    #[error("randomness coefficient overflow")]
    RandomnessOverflow,
    #[error("randomness coefficient outside [-beta, beta]")]
    RandomnessOutOfRange,
    #[error("witness does not match expected commitment")]
    CommitmentMismatch,
    #[error("invalid proof options for the requested proof role")]
    ProofOptionsMismatch,
    #[error("invalid public input length: expected {expected}, got {actual}")]
    InvalidPublicInputLength { expected: usize, actual: usize },
    #[error("transfer beta must be positive and at most balance beta")]
    InvalidBetaBounds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DualBetaBounds {
    pub transfer_randomness_beta: i64,
    pub balance_randomness_beta: i64,
}

impl DualBetaBounds {
    pub fn new(
        transfer_randomness_beta: i64,
        balance_randomness_beta: i64,
    ) -> Result<Self, ProofAdapterError> {
        if transfer_randomness_beta <= 0
            || balance_randomness_beta <= 0
            || transfer_randomness_beta > balance_randomness_beta
        {
            return Err(ProofAdapterError::InvalidBetaBounds);
        }
        Ok(Self {
            transfer_randomness_beta,
            balance_randomness_beta,
        })
    }

    pub fn recommended_from_balance_randomness_beta(
        balance_randomness_beta: i64,
    ) -> Result<Self, ProofAdapterError> {
        let transfer_randomness_beta = core::cmp::max(1, balance_randomness_beta / 10_000);
        Self::new(transfer_randomness_beta, balance_randomness_beta)
    }

    pub fn default_from_upstream() -> Self {
        Self::recommended_from_balance_randomness_beta(sis_amount_stark::DEFAULT_BETA)
            .expect("upstream default beta must produce valid derived bounds")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HiddenAmountWitness {
    pub amount: u64,
    pub randomness: LatticeRandomness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferAmountProof {
    pub envelope_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BalanceProof {
    pub envelope_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedHiddenAmountProof {
    pub commitment: LatticeCommitment,
    pub options: ProofSystemOptions,
}

pub struct DualBetaPreparedSystems {
    bounds: DualBetaBounds,
    transfer_options: ProofSystemOptions,
    balance_options: ProofSystemOptions,
    transfer_prepared: PreparedSystem,
    balance_prepared: PreparedSystem,
}

impl DualBetaPreparedSystems {
    pub fn bounds(&self) -> DualBetaBounds {
        self.bounds
    }

    pub fn transfer_options(&self) -> &ProofSystemOptions {
        &self.transfer_options
    }

    pub fn balance_options(&self) -> &ProofSystemOptions {
        &self.balance_options
    }
}

pub fn prepare_dual_beta_systems(
    bounds: DualBetaBounds,
) -> Result<DualBetaPreparedSystems, ProofAdapterError> {
    let transfer_options = ProofSystemOptions {
        beta: bounds.transfer_randomness_beta,
        ..ProofSystemOptions::default()
    };
    let balance_options = ProofSystemOptions {
        beta: bounds.balance_randomness_beta,
        ..ProofSystemOptions::default()
    };
    let transfer_prepared = prepare_system(&transfer_options)?;
    let balance_prepared = prepare_system(&balance_options)?;
    Ok(DualBetaPreparedSystems {
        bounds,
        transfer_options,
        balance_options,
        transfer_prepared,
        balance_prepared,
    })
}

pub fn validate_randomness_for_beta(
    randomness: &LatticeRandomness,
    beta: i64,
) -> Result<(), ProofAdapterError> {
    if randomness
        .iter()
        .any(|value| !(-beta..=beta).contains(value))
    {
        return Err(ProofAdapterError::RandomnessOutOfRange);
    }
    Ok(())
}

pub fn compute_lattice_commitment(
    amount: u64,
    randomness: &LatticeRandomness,
) -> LatticeCommitment {
    compute_commitment(amount, randomness)
}

fn public_inputs_to_commitment(
    public_inputs: &PublicInputs,
) -> Result<LatticeCommitment, ProofAdapterError> {
    if public_inputs.c.len() != sis_amount_stark::params::M {
        return Err(ProofAdapterError::InvalidPublicInputLength {
            expected: sis_amount_stark::params::M,
            actual: public_inputs.c.len(),
        });
    }
    public_inputs.c.clone().try_into().map_err(|vec: Vec<u64>| {
        ProofAdapterError::InvalidPublicInputLength {
            expected: sis_amount_stark::params::M,
            actual: vec.len(),
        }
    })
}

fn prove_with_prepared(
    witness: &HiddenAmountWitness,
    prepared: &PreparedSystem,
    options: &ProofSystemOptions,
) -> Result<Vec<u8>, ProofAdapterError> {
    validate_randomness_for_beta(&witness.randomness, options.beta)?;
    let (proof, public_inputs) =
        prove_amount_prepared(prepared, witness.amount, witness.randomness)?;
    Ok(serialize_envelope(&proof, &public_inputs, options)?)
}

fn verify_with_prepared(
    envelope_bytes: &[u8],
    prepared: &PreparedSystem,
    expected_options: &ProofSystemOptions,
) -> Result<VerifiedHiddenAmountProof, ProofAdapterError> {
    let (proof, public_inputs, options) = deserialize_envelope(envelope_bytes)?;
    if &options != expected_options {
        return Err(ProofAdapterError::ProofOptionsMismatch);
    }
    verify_amount_prepared(prepared, &proof, &public_inputs)?;
    Ok(VerifiedHiddenAmountProof {
        commitment: public_inputs_to_commitment(&public_inputs)?,
        options,
    })
}

pub fn prove_transfer_amount(
    systems: &DualBetaPreparedSystems,
    witness: &HiddenAmountWitness,
) -> Result<TransferAmountProof, ProofAdapterError> {
    Ok(TransferAmountProof {
        envelope_bytes: prove_with_prepared(
            witness,
            &systems.transfer_prepared,
            &systems.transfer_options,
        )?,
    })
}

pub fn verify_transfer_amount_proof(
    systems: &DualBetaPreparedSystems,
    proof: &TransferAmountProof,
) -> Result<VerifiedHiddenAmountProof, ProofAdapterError> {
    verify_with_prepared(
        &proof.envelope_bytes,
        &systems.transfer_prepared,
        &systems.transfer_options,
    )
}

pub fn prove_balance_opening(
    systems: &DualBetaPreparedSystems,
    witness: &HiddenAmountWitness,
) -> Result<BalanceProof, ProofAdapterError> {
    Ok(BalanceProof {
        envelope_bytes: prove_with_prepared(
            witness,
            &systems.balance_prepared,
            &systems.balance_options,
        )?,
    })
}

pub fn verify_balance_proof(
    systems: &DualBetaPreparedSystems,
    proof: &BalanceProof,
) -> Result<VerifiedHiddenAmountProof, ProofAdapterError> {
    verify_with_prepared(
        &proof.envelope_bytes,
        &systems.balance_prepared,
        &systems.balance_options,
    )
}

pub fn verify_witness_matches_commitment(
    witness: &HiddenAmountWitness,
    expected: &LatticeCommitment,
) -> Result<(), ProofAdapterError> {
    let actual = compute_lattice_commitment(witness.amount, &witness.randomness);
    if actual != *expected {
        return Err(ProofAdapterError::CommitmentMismatch);
    }
    Ok(())
}

pub fn add_commitments(lhs: &LatticeCommitment, rhs: &LatticeCommitment) -> LatticeCommitment {
    let mut out = [0_u64; sis_amount_stark::params::M];
    for i in 0..sis_amount_stark::params::M {
        out[i] = (lhs[i] + rhs[i]) % sis_amount_stark::params::Q;
    }
    out
}

pub fn sub_commitments(lhs: &LatticeCommitment, rhs: &LatticeCommitment) -> LatticeCommitment {
    let mut out = [0_u64; sis_amount_stark::params::M];
    for i in 0..sis_amount_stark::params::M {
        let q = sis_amount_stark::params::Q;
        out[i] = (lhs[i] + q - (rhs[i] % q)) % q;
    }
    out
}

fn add_randomness(
    lhs: &LatticeRandomness,
    rhs: &LatticeRandomness,
    beta: i64,
) -> Result<LatticeRandomness, ProofAdapterError> {
    let mut out = [0_i64; sis_amount_stark::params::N];
    for i in 0..sis_amount_stark::params::N {
        out[i] = lhs[i]
            .checked_add(rhs[i])
            .ok_or(ProofAdapterError::RandomnessOverflow)?;
    }
    validate_randomness_for_beta(&out, beta)?;
    Ok(out)
}

fn sub_randomness(
    lhs: &LatticeRandomness,
    rhs: &LatticeRandomness,
    beta: i64,
) -> Result<LatticeRandomness, ProofAdapterError> {
    let mut out = [0_i64; sis_amount_stark::params::N];
    for i in 0..sis_amount_stark::params::N {
        out[i] = lhs[i]
            .checked_sub(rhs[i])
            .ok_or(ProofAdapterError::RandomnessOverflow)?;
    }
    validate_randomness_for_beta(&out, beta)?;
    Ok(out)
}

pub fn apply_amount_to_receiver_balance(
    balance_before: &HiddenAmountWitness,
    amount_witness: &HiddenAmountWitness,
    balance_randomness_beta: i64,
) -> Result<HiddenAmountWitness, ProofAdapterError> {
    let amount = balance_before
        .amount
        .checked_add(amount_witness.amount)
        .ok_or(ProofAdapterError::AmountOverflow)?;
    let randomness = add_randomness(
        &balance_before.randomness,
        &amount_witness.randomness,
        balance_randomness_beta,
    )?;
    Ok(HiddenAmountWitness { amount, randomness })
}

pub fn apply_amount_to_sender_balance(
    balance_before: &HiddenAmountWitness,
    amount_witness: &HiddenAmountWitness,
    balance_randomness_beta: i64,
) -> Result<HiddenAmountWitness, ProofAdapterError> {
    let amount = balance_before
        .amount
        .checked_sub(amount_witness.amount)
        .ok_or(ProofAdapterError::AmountUnderflow)?;
    let randomness = sub_randomness(
        &balance_before.randomness,
        &amount_witness.randomness,
        balance_randomness_beta,
    )?;
    Ok(HiddenAmountWitness { amount, randomness })
}

#[cfg(test)]
mod tests {
    use super::{
        BalanceProof, DualBetaBounds, HiddenAmountWitness, TransferAmountProof, add_commitments,
        apply_amount_to_receiver_balance, apply_amount_to_sender_balance,
        compute_lattice_commitment, prepare_dual_beta_systems, prove_balance_opening,
        prove_transfer_amount, sub_commitments, validate_randomness_for_beta, verify_balance_proof,
        verify_transfer_amount_proof, verify_witness_matches_commitment,
    };
    use channel_types::LatticeRandomness;
    use sis_amount_stark::compute_commitment;

    fn randomness_from_prefix(prefix: &[i64]) -> LatticeRandomness {
        let mut r: LatticeRandomness = core::array::from_fn(|_| 0_i64);
        for (idx, value) in prefix.iter().enumerate() {
            r[idx] = *value;
        }
        r
    }

    #[test]
    fn transfer_amount_roundtrip_verifies_with_small_beta() {
        let bounds = DualBetaBounds::new(31, 200_000).expect("valid bounds");
        let systems = prepare_dual_beta_systems(bounds).expect("systems should prepare");
        let witness = HiddenAmountWitness {
            amount: 321,
            randomness: randomness_from_prefix(&[4, -1, 0, 7]),
        };

        let proof = prove_transfer_amount(&systems, &witness).expect("proof should be created");
        let verified = verify_transfer_amount_proof(&systems, &proof).expect("proof should verify");

        assert_eq!(
            verified.commitment,
            compute_lattice_commitment(witness.amount, &witness.randomness)
        );
    }

    #[test]
    fn balance_roundtrip_verifies_with_large_beta() {
        let bounds = DualBetaBounds::new(31, 200_000).expect("valid bounds");
        let systems = prepare_dual_beta_systems(bounds).expect("systems should prepare");
        let witness = HiddenAmountWitness {
            amount: 321,
            randomness: randomness_from_prefix(&[100_000, -1, 0, 7]),
        };

        let proof = prove_balance_opening(&systems, &witness).expect("proof should be created");
        let verified = verify_balance_proof(&systems, &proof).expect("proof should verify");

        assert_eq!(
            verified.commitment,
            compute_lattice_commitment(witness.amount, &witness.randomness)
        );
    }

    #[test]
    fn transfer_proof_rejects_balance_bound_proof() {
        let bounds = DualBetaBounds::new(31, 200_000).expect("valid bounds");
        let systems = prepare_dual_beta_systems(bounds).expect("systems should prepare");
        let witness = HiddenAmountWitness {
            amount: 321,
            randomness: randomness_from_prefix(&[4, -1, 0, 7]),
        };

        let balance_like =
            prove_balance_opening(&systems, &witness).expect("proof should be created");
        let transfer_like = TransferAmountProof {
            envelope_bytes: balance_like.envelope_bytes,
        };

        assert!(verify_transfer_amount_proof(&systems, &transfer_like).is_err());
    }

    #[test]
    fn malformed_proof_is_rejected() {
        let bounds = DualBetaBounds::new(31, 200_000).expect("valid bounds");
        let systems = prepare_dual_beta_systems(bounds).expect("systems should prepare");
        let witness = HiddenAmountWitness {
            amount: 321,
            randomness: randomness_from_prefix(&[4, -1, 0, 7]),
        };

        let mut proof = prove_transfer_amount(&systems, &witness).expect("proof should be created");
        proof.envelope_bytes[0] ^= 0x01;

        assert!(verify_transfer_amount_proof(&systems, &proof).is_err());
    }

    #[test]
    fn adapter_commitment_matches_upstream() {
        let randomness = randomness_from_prefix(&[3, 1, -2, 0]);
        assert_eq!(
            compute_lattice_commitment(999, &randomness),
            compute_commitment(999, &randomness)
        );
    }

    #[test]
    fn commitment_arithmetic_matches_opening_arithmetic() {
        let bounds = DualBetaBounds::new(31, 200_000).expect("valid bounds");
        let balance_before = HiddenAmountWitness {
            amount: 500,
            randomness: randomness_from_prefix(&[5, 5, 5, 5]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 120,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };
        let sender_after = apply_amount_to_sender_balance(
            &balance_before,
            &amount_witness,
            bounds.balance_randomness_beta,
        )
        .expect("sender update");
        let receiver_after = apply_amount_to_receiver_balance(
            &balance_before,
            &amount_witness,
            bounds.balance_randomness_beta,
        )
        .expect("receiver update");

        assert_eq!(
            compute_lattice_commitment(sender_after.amount, &sender_after.randomness),
            sub_commitments(
                &compute_lattice_commitment(balance_before.amount, &balance_before.randomness),
                &compute_lattice_commitment(amount_witness.amount, &amount_witness.randomness),
            )
        );
        assert_eq!(
            compute_lattice_commitment(receiver_after.amount, &receiver_after.randomness),
            add_commitments(
                &compute_lattice_commitment(balance_before.amount, &balance_before.randomness),
                &compute_lattice_commitment(amount_witness.amount, &amount_witness.randomness),
            )
        );
    }

    #[test]
    fn witness_match_check_rejects_mismatch() {
        let witness = HiddenAmountWitness {
            amount: 100,
            randomness: randomness_from_prefix(&[1, 0, 0, 0]),
        };
        let wrong_commitment =
            compute_lattice_commitment(101, &randomness_from_prefix(&[1, 0, 0, 0]));

        assert!(verify_witness_matches_commitment(&witness, &wrong_commitment).is_err());
    }

    #[test]
    fn randomness_bound_is_enforced() {
        assert!(
            validate_randomness_for_beta(&randomness_from_prefix(&[1_000, 0, 0, 0]), 999).is_err()
        );
    }

    #[test]
    fn sender_update_rejects_underflow() {
        let balance_before = HiddenAmountWitness {
            amount: 10,
            randomness: randomness_from_prefix(&[1, 1, 1, 1]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 20,
            randomness: randomness_from_prefix(&[1, 0, 0, 0]),
        };
        assert!(apply_amount_to_sender_balance(&balance_before, &amount_witness, 10_000).is_err());
    }

    #[test]
    fn receiver_update_rejects_randomness_out_of_range() {
        let balance_before = HiddenAmountWitness {
            amount: 10,
            randomness: randomness_from_prefix(&[200_000, 0, 0, 0]),
        };
        let amount_witness = HiddenAmountWitness {
            amount: 1,
            randomness: randomness_from_prefix(&[1, 0, 0, 0]),
        };
        assert!(
            apply_amount_to_receiver_balance(&balance_before, &amount_witness, 200_000).is_err()
        );
    }

    #[test]
    fn proof_wrapper_types_hold_bytes() {
        let transfer = TransferAmountProof {
            envelope_bytes: vec![1, 2, 3],
        };
        let balance = BalanceProof {
            envelope_bytes: vec![4, 5, 6],
        };
        assert_eq!(transfer.envelope_bytes, vec![1, 2, 3]);
        assert_eq!(balance.envelope_bytes, vec![4, 5, 6]);
    }
}
