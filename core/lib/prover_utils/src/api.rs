use serde::{Deserialize, Serialize};
use zksync_basic_types::BlockNumber;
use zksync_circuit::serialization::ProverData;
use zksync_crypto::proof::{AggregatedProof, SingleProof};

pub type ProverId = String;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProverInputRequest {
    pub prover_name: ProverId,
    pub aux_data: ProverInputRequestAuxData,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProverInputRequestAuxData {
    pub prefer_aggregated_proof: Option<bool>,
    pub preferred_block_size: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProverInputResponse {
    pub job_id: i32,
    pub first_block: BlockNumber,
    pub last_block: BlockNumber,
    pub data: Option<JobRequestData>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobRequestData {
    BlockProof(
        ProverData,
        // zksync_circuit::circuit::ZkSyncCircuit<'static, Engine>,
        usize, // block size
    ),
    AggregatedBlockProof(Vec<(SingleProof, usize)>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProverOutputRequest {
    pub job_id: i32,
    pub first_block: BlockNumber,
    pub last_block: BlockNumber,
    pub data: JobResultData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JobResultData {
    BlockProof(SingleProof),
    AggregatedBlockProof(AggregatedProof),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkingOn {
    pub prover_name: String,
    pub job_id: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProverStopped {
    pub prover_id: ProverId,
}
