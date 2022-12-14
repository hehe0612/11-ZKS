use chrono::{DateTime, Utc};
use std::cmp::max;
use std::time::Duration;
use zksync_crypto::proof::AggregatedProof;
use zksync_storage::chain::block::BlockSchema;
use zksync_storage::chain::operations::OperationsSchema;
use zksync_storage::prover::ProverSchema;
use zksync_storage::StorageProcessor;
use zksync_types::aggregated_operations::{
    AggregatedActionType, AggregatedOperation, BlocksCommitOperation, BlocksCreateProofOperation,
    BlocksExecuteOperation, BlocksProofOperation,
};
use zksync_types::block::Block;
use zksync_types::U256;

fn create_new_commit_operation(
    last_committed_block: &Block,
    new_blocks: &[Block],
    current_time: DateTime<Utc>,
    max_blocks_to_commit: usize,
    block_commit_deadline: Duration,
    max_gas_for_tx: U256,
) -> Option<BlocksCommitOperation> {
    let any_block_commit_deadline_triggered = {
        let block_commit_deadline_seconds = block_commit_deadline.as_secs() as i64;
        new_blocks
            .iter()
            .take(max_blocks_to_commit)
            .find(|block| {
                let seconds_since_block_created = max(
                    current_time
                        // todo: block timestamp?
                        .signed_duration_since(block.timestamp_utc())
                        .num_seconds(),
                    0,
                );
                seconds_since_block_created > block_commit_deadline_seconds
            })
            .is_some()
    };

    let gas_limit_reached_for_blocks = new_blocks
        .iter()
        .take(max_blocks_to_commit)
        .map(|block| block.commit_gas_limit.as_u64())
        .sum::<u64>()
        >= max_gas_for_tx.as_u64();

    let should_commit_blocks = any_block_commit_deadline_triggered
        || gas_limit_reached_for_blocks
        || new_blocks.len() == max_blocks_to_commit;
    if !should_commit_blocks {
        return None;
    }

    let mut blocks_to_commit = Vec::new();
    let mut commit_tx_gas = U256::from(0);
    for new_block in new_blocks.iter().take(max_blocks_to_commit) {
        if commit_tx_gas + new_block.commit_gas_limit > max_gas_for_tx {
            break;
        }
        blocks_to_commit.push(new_block.clone());
        commit_tx_gas += new_block.commit_gas_limit;
    }
    assert!(blocks_to_commit.len() > 0);

    Some(BlocksCommitOperation {
        last_committed_block: last_committed_block.clone(),
        blocks: blocks_to_commit,
    })
}

fn create_new_create_proof_operation(
    new_blocks_with_proofs: &[Block],
    available_aggregate_proof_sizes: &[usize],
    current_time: DateTime<Utc>,
    block_verify_deadline: Duration,
    _max_gas_for_tx: U256,
) -> Option<BlocksCreateProofOperation> {
    let max_aggregate_size = available_aggregate_proof_sizes
        .last()
        .cloned()
        .expect("should have at least one aggregate proof size");

    let any_block_verify_deadline_triggered = {
        let block_verify_deadline = block_verify_deadline.as_secs() as i64;
        new_blocks_with_proofs
            .iter()
            .take(max_aggregate_size)
            .find(|block| {
                let seconds_since_block_created = max(
                    current_time
                        .signed_duration_since(block.timestamp_utc())
                        .num_seconds(),
                    0,
                );
                seconds_since_block_created > block_verify_deadline
            })
            .is_some()
    };

    let can_create_max_aggregate_proof = new_blocks_with_proofs.len() >= max_aggregate_size;

    let should_create_aggregate_proof =
        any_block_verify_deadline_triggered || can_create_max_aggregate_proof;

    if !should_create_aggregate_proof {
        return None;
    }

    // get max possible aggregate size
    let aggregate_proof_size = available_aggregate_proof_sizes
        .iter()
        .rev()
        .find(|aggregate_size| *aggregate_size >= &new_blocks_with_proofs.len())
        .cloned()
        .expect("failed to find correct aggregate proof size");

    let blocks = new_blocks_with_proofs
        .iter()
        .take(aggregate_proof_size)
        .cloned()
        .collect::<Vec<_>>();

    let proofs_to_pad = aggregate_proof_size
        .checked_sub(blocks.len())
        .expect("incorrect aggregate proof size");

    Some(BlocksCreateProofOperation {
        blocks,
        proofs_to_pad,
    })
}

fn create_publish_proof_operation(
    unpublished_create_proof_op: &BlocksCreateProofOperation,
    aggregated_proof: &AggregatedProof,
) -> BlocksProofOperation {
    BlocksProofOperation {
        blocks: unpublished_create_proof_op.blocks.clone(),
        proof: aggregated_proof.serialize_aggregated_proof(),
    }
}

fn create_execute_blocks_operation(
    proven_non_executed_block: &[Block],
    current_time: DateTime<Utc>,
    max_blocks_to_execute: usize,
    block_execute_deadline: Duration,
    max_gas_for_tx: U256,
) -> Option<BlocksExecuteOperation> {
    let any_block_execute_deadline_triggered = {
        let block_execute_deadline_seconds = block_execute_deadline.as_secs() as i64;
        proven_non_executed_block
            .iter()
            .take(max_blocks_to_execute)
            .find(|block| {
                let seconds_since_block_created = max(
                    current_time
                        .signed_duration_since(block.timestamp_utc())
                        .num_seconds(),
                    0,
                );
                seconds_since_block_created > block_execute_deadline_seconds
            })
            .is_some()
    };

    let gas_limit_reached_for_blocks = proven_non_executed_block
        .iter()
        .take(max_blocks_to_execute)
        .map(|block| block.verify_gas_limit.as_u64())
        .sum::<u64>()
        >= max_gas_for_tx.as_u64();

    let should_execute_blocks = any_block_execute_deadline_triggered
        || gas_limit_reached_for_blocks
        || proven_non_executed_block.len() == max_blocks_to_execute;
    if !should_execute_blocks {
        return None;
    }

    let mut blocks_to_execute = Vec::new();
    let mut execute_tx_gas = U256::from(0);
    for block in proven_non_executed_block.iter().take(max_blocks_to_execute) {
        if execute_tx_gas + block.verify_gas_limit > max_gas_for_tx {
            break;
        }
        blocks_to_execute.push(block.clone());
        execute_tx_gas += block.verify_gas_limit;
    }
    assert!(blocks_to_execute.len() > 0);

    Some(BlocksExecuteOperation {
        blocks: blocks_to_execute,
    })
}

const MAX_BLOCK_TO_COMMIT: usize = 5;
const BLOCK_COMMIT_DEADLINE: Duration = Duration::from_secs(10);
const MAX_GAS_TX: u64 = 2_000_000;
const AVAILABLE_AGGREGATE_PROOFS: &[usize] = &[1, 5];

async fn create_aggregated_commits_storage(
    storage: &mut StorageProcessor<'_>,
) -> anyhow::Result<bool> {
    let last_committed_block = BlockSchema(storage).get_last_committed_block().await?;
    let last_aggregate_committed_block = OperationsSchema(storage)
        .get_last_affected_block_by_aggregated_action(AggregatedActionType::CommitBlocks)
        .await?;
    if last_committed_block <= last_aggregate_committed_block {
        return Ok(false);
    }

    let old_committed_block = BlockSchema(storage)
        .get_block(last_aggregate_committed_block)
        .await?
        .expect("Failed to get last committed block from db");
    let mut new_blocks = Vec::new();
    for block_number in last_aggregate_committed_block + 1..=last_committed_block {
        let block = BlockSchema(storage)
            .get_block(block_number)
            .await?
            .expect("Failed to get committed block");
        new_blocks.push(block);
    }

    let commit_operation = create_new_commit_operation(
        &old_committed_block,
        &new_blocks,
        Utc::now(),
        MAX_BLOCK_TO_COMMIT,
        BLOCK_COMMIT_DEADLINE,
        MAX_GAS_TX.into(),
    );

    if let Some(commit_operation) = commit_operation {
        let aggregated_op = commit_operation.into();
        log_aggregated_op_creation(&aggregated_op);
        OperationsSchema(storage)
            .store_aggregated_action(aggregated_op)
            .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn create_aggregated_prover_task_storage(
    storage: &mut StorageProcessor<'_>,
) -> anyhow::Result<bool> {
    let last_committed_block = BlockSchema(storage).get_last_committed_block().await?;
    let last_aggregate_create_proof_block = OperationsSchema(storage)
        .get_last_affected_block_by_aggregated_action(AggregatedActionType::CreateProofBlocks)
        .await?;
    if last_committed_block <= last_aggregate_create_proof_block {
        return Ok(false);
    }

    let mut blocks_with_proofs = Vec::new();
    for block_number in last_aggregate_create_proof_block + 1..=last_committed_block {
        let proof_exists = ProverSchema(storage)
            .load_proof(block_number)
            .await?
            .is_some();
        if proof_exists {
            let block = BlockSchema(storage)
                .get_block(block_number)
                .await?
                .expect("failed to fetch committed block from db");
            blocks_with_proofs.push(block);
        } else {
            break;
        }
    }

    let create_proof_operation = create_new_create_proof_operation(
        &blocks_with_proofs,
        AVAILABLE_AGGREGATE_PROOFS,
        Utc::now(),
        BLOCK_COMMIT_DEADLINE,
        MAX_GAS_TX.into(),
    );
    if let Some(operation) = create_proof_operation {
        let aggregated_op = operation.into();
        log_aggregated_op_creation(&aggregated_op);
        OperationsSchema(storage)
            .store_aggregated_action(aggregated_op)
            .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn create_aggregated_publish_proof_operation_storage(
    storage: &mut StorageProcessor<'_>,
) -> anyhow::Result<bool> {
    let last_aggregate_create_proof_block = OperationsSchema(storage)
        .get_last_affected_block_by_aggregated_action(AggregatedActionType::CreateProofBlocks)
        .await?;
    let last_aggregate_publish_proof_block = OperationsSchema(storage)
        .get_last_affected_block_by_aggregated_action(
            AggregatedActionType::PublishProofBlocksOnchain,
        )
        .await?;
    if last_aggregate_create_proof_block <= last_aggregate_publish_proof_block {
        return Ok(false);
    }

    let last_unpublished_create_proof_operation = {
        let (_, aggregated_operation) = OperationsSchema(storage)
            .get_aggregated_op_that_affects_block(
                AggregatedActionType::CreateProofBlocks,
                last_aggregate_publish_proof_block + 1,
            )
            .await?
            .expect("Unpublished create proof operation should exist");
        if let AggregatedOperation::CreateProofBlocks(create_proof_blocks) = aggregated_operation {
            create_proof_blocks
        } else {
            panic!("Incorrect aggregate operation type")
        }
    };

    let aggregated_proof = {
        assert!(
            last_unpublished_create_proof_operation.blocks.len() > 0,
            "should have 1 block"
        );
        let first_block = last_unpublished_create_proof_operation
            .blocks
            .first()
            .map(|b| b.block_number)
            .unwrap();
        let last_block = last_unpublished_create_proof_operation
            .blocks
            .last()
            .map(|b| b.block_number)
            .unwrap();
        storage
            .prover_schema()
            .load_aggregated_proof(first_block, last_block)
            .await?
    };

    if let Some(proof) = aggregated_proof {
        let operation =
            create_publish_proof_operation(&last_unpublished_create_proof_operation, &proof);
        let aggregated_op = operation.into();
        log_aggregated_op_creation(&aggregated_op);
        OperationsSchema(storage)
            .store_aggregated_action(aggregated_op)
            .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

async fn create_aggregated_execute_operation_storage(
    storage: &mut StorageProcessor<'_>,
) -> anyhow::Result<bool> {
    let last_aggregate_executed_block = OperationsSchema(storage)
        .get_last_affected_block_by_aggregated_action(AggregatedActionType::ExecuteBlocks)
        .await?;
    let last_aggregate_publish_proof_block = OperationsSchema(storage)
        .get_last_affected_block_by_aggregated_action(
            AggregatedActionType::PublishProofBlocksOnchain,
        )
        .await?;

    if last_aggregate_publish_proof_block <= last_aggregate_executed_block {
        return Ok(false);
    }

    let mut blocks = Vec::new();
    for block_number in last_aggregate_executed_block + 1..=last_aggregate_publish_proof_block {
        let block = BlockSchema(storage)
            .get_block(block_number)
            .await?
            .expect("Failed to get block that should be committed");
        blocks.push(block);
    }

    let execute_operation = create_execute_blocks_operation(
        &blocks,
        Utc::now(),
        MAX_BLOCK_TO_COMMIT,
        BLOCK_COMMIT_DEADLINE,
        MAX_GAS_TX.into(),
    );

    if let Some(operation) = execute_operation {
        let aggregated_op = operation.into();
        log_aggregated_op_creation(&aggregated_op);
        OperationsSchema(storage)
            .store_aggregated_action(aggregated_op)
            .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn create_aggregated_operations_storage(
    storage: &mut StorageProcessor<'_>,
) -> anyhow::Result<()> {
    while create_aggregated_commits_storage(storage).await? {}
    while create_aggregated_prover_task_storage(storage).await? {}
    while create_aggregated_publish_proof_operation_storage(storage).await? {}
    while create_aggregated_execute_operation_storage(storage).await? {}

    Ok(())
}

fn log_aggregated_op_creation(aggregated_op: &AggregatedOperation) {
    let (first, last) = aggregated_op.get_block_range();
    log::info!(
        "Created aggregated operation: {}, blocks: [{},{}]",
        aggregated_op.get_action_type().to_string(),
        first,
        last
    );
}
