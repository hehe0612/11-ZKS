// Built-in deps
use std::time::Instant;
// External imports
// Workspace imports
use zksync_types::{tx::TxHash, ActionType, BlockNumber};
// Local imports
use self::records::{
    NewExecutedPriorityOperation, NewExecutedTransaction, NewOperation, StoredAggregatedOperation,
    StoredExecutedPriorityOperation, StoredOperation,
};
use crate::chain::operations::records::StoredExecutedTransaction;
use crate::chain::operations_ext::OperationsExtSchema;
use crate::ethereum::EthereumSchema;
use crate::{chain::mempool::MempoolSchema, QueryResult, StorageProcessor};
use zksync_basic_types::H256;
use zksync_types::aggregated_operations::{AggregatedActionType, AggregatedOperation};

pub mod records;

/// Operations schema is capable of storing and loading the transactions.
/// Every kind of transaction (non-executed, executed, and executed priority tx)
/// can be either saved or loaded from the database.
#[derive(Debug)]
pub struct OperationsSchema<'a, 'c>(pub &'a mut StorageProcessor<'c>);

impl<'a, 'c> OperationsSchema<'a, 'c> {
    pub async fn get_last_block_by_action(
        &mut self,
        action_type: ActionType,
        confirmed: Option<bool>,
    ) -> QueryResult<BlockNumber> {
        let start = Instant::now();
        let max_block = sqlx::query!(
            r#"SELECT max(block_number) FROM operations WHERE action_type = $1 AND confirmed IS DISTINCT FROM $2"#,
            action_type.to_string(),
            confirmed.map(|value| !value)
        )
        .fetch_one(self.0.conn())
        .await?
        .max
        .unwrap_or(0);

        metrics::histogram!(
            "sql.chain.operations.get_last_block_by_action",
            start.elapsed()
        );
        Ok(max_block as BlockNumber)
    }

    pub async fn get_operation(
        &mut self,
        block_number: BlockNumber,
        action_type: ActionType,
    ) -> Option<StoredOperation> {
        let start = Instant::now();
        let result = sqlx::query_as!(
            StoredOperation,
            "SELECT * FROM operations WHERE block_number = $1 AND action_type = $2",
            i64::from(block_number),
            action_type.to_string()
        )
        .fetch_optional(self.0.conn())
        .await
        .ok()
        .flatten();

        metrics::histogram!("sql.chain.operations.get_operation", start.elapsed());
        result
    }

    pub async fn get_executed_operation(
        &mut self,
        op_hash: &[u8],
    ) -> QueryResult<Option<StoredExecutedTransaction>> {
        let start = Instant::now();
        let op = sqlx::query_as!(
            StoredExecutedTransaction,
            "SELECT * FROM executed_transactions WHERE tx_hash = $1",
            op_hash
        )
        .fetch_optional(self.0.conn())
        .await?;

        metrics::histogram!(
            "sql.chain.operations.get_executed_operation",
            start.elapsed()
        );
        Ok(op)
    }

    pub async fn get_executed_priority_operation(
        &mut self,
        priority_op_id: u32,
    ) -> QueryResult<Option<StoredExecutedPriorityOperation>> {
        let start = Instant::now();
        let op = sqlx::query_as!(
            StoredExecutedPriorityOperation,
            "SELECT * FROM executed_priority_operations WHERE priority_op_serialid = $1",
            i64::from(priority_op_id)
        )
        .fetch_optional(self.0.conn())
        .await?;

        metrics::histogram!(
            "sql.chain.operations.get_executed_priority_operation",
            start.elapsed()
        );
        Ok(op)
    }

    pub async fn get_executed_priority_operation_by_hash(
        &mut self,
        eth_hash: &[u8],
    ) -> QueryResult<Option<StoredExecutedPriorityOperation>> {
        let start = Instant::now();
        let op = sqlx::query_as!(
            StoredExecutedPriorityOperation,
            "SELECT * FROM executed_priority_operations WHERE eth_hash = $1",
            eth_hash
        )
        .fetch_optional(self.0.conn())
        .await?;

        metrics::histogram!(
            "sql.chain.operations.get_executed_priority_operation_by_hash",
            start.elapsed()
        );
        Ok(op)
    }

    pub(crate) async fn store_operation(
        &mut self,
        operation: NewOperation,
    ) -> QueryResult<StoredOperation> {
        let start = Instant::now();
        let op = sqlx::query_as!(
            StoredOperation,
            "INSERT INTO operations (block_number, action_type) VALUES ($1, $2)
            RETURNING *",
            operation.block_number,
            operation.action_type
        )
        .fetch_one(self.0.conn())
        .await?;
        metrics::histogram!("sql.chain.operations.store_operation", start.elapsed());
        Ok(op)
    }

    pub(crate) async fn confirm_operation(
        &mut self,
        block_number: BlockNumber,
        action_type: ActionType,
    ) -> QueryResult<()> {
        let start = Instant::now();
        sqlx::query!(
            "UPDATE operations
                SET confirmed = $1
                WHERE block_number = $2 AND action_type = $3",
            true,
            i64::from(block_number),
            action_type.to_string()
        )
        .execute(self.0.conn())
        .await?;
        metrics::histogram!("sql.chain.operations.confirm_operation", start.elapsed());
        Ok(())
    }

    pub async fn confirm_operations(
        &mut self,
        first_block: BlockNumber,
        last_block: BlockNumber,
        action_type: ActionType,
    ) -> QueryResult<()> {
        let start = Instant::now();
        sqlx::query!(
            "UPDATE operations
                SET confirmed = $1
                WHERE block_number >= $2 AND block_number <= $3 AND action_type = $4",
            true,
            i64::from(first_block),
            i64::from(last_block),
            action_type.to_string()
        )
        .execute(self.0.conn())
        .await?;
        metrics::histogram!("sql.chain.operations.confirm_operations", start.elapsed());
        Ok(())
    }

    /// Stores the executed transaction in the database.
    pub(crate) async fn store_executed_tx(
        &mut self,
        operation: NewExecutedTransaction,
    ) -> QueryResult<()> {
        let start = Instant::now();
        let mut transaction = self.0.start_transaction().await?;

        MempoolSchema(&mut transaction)
            .remove_tx(&operation.tx_hash)
            .await?;

        if operation.success {
            // If transaction succeed, it should replace the stored tx with the same hash.
            // The situation when a duplicate tx is stored in the database may exist only if has
            // failed previously.
            // Possible scenario: user had no enough funds for transfer, then deposited some and
            // sent the same transfer again.

            sqlx::query!(
                "INSERT INTO executed_transactions (block_number, block_index, tx, operation, tx_hash, from_account, to_account, success, fail_reason, primary_account_address, nonce, created_at, eth_sign_data, batch_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                ON CONFLICT (tx_hash)
                DO UPDATE
                SET block_number = $1, block_index = $2, tx = $3, operation = $4, tx_hash = $5, from_account = $6, to_account = $7, success = $8, fail_reason = $9, primary_account_address = $10, nonce = $11, created_at = $12, eth_sign_data = $13, batch_id = $14",
                operation.block_number,
                operation.block_index,
                operation.tx,
                operation.operation,
                operation.tx_hash,
                operation.from_account,
                operation.to_account,
                operation.success,
                operation.fail_reason,
                operation.primary_account_address,
                operation.nonce,
                operation.created_at,
                operation.eth_sign_data,
                operation.batch_id,
            )
            .execute(transaction.conn())
            .await?;
        } else {
            // If transaction failed, we do nothing on conflict.
            sqlx::query!(
                "INSERT INTO executed_transactions (block_number, block_index, tx, operation, tx_hash, from_account, to_account, success, fail_reason, primary_account_address, nonce, created_at, eth_sign_data, batch_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                ON CONFLICT (tx_hash)
                DO NOTHING",
                operation.block_number,
                operation.block_index,
                operation.tx,
                operation.operation,
                operation.tx_hash,
                operation.from_account,
                operation.to_account,
                operation.success,
                operation.fail_reason,
                operation.primary_account_address,
                operation.nonce,
                operation.created_at,
                operation.eth_sign_data,
                operation.batch_id,
            )
            .execute(transaction.conn())
            .await?;
        };

        transaction.commit().await?;
        metrics::histogram!("sql.chain.operations.store_executed_tx", start.elapsed());
        Ok(())
    }

    /// Stores executed priority operation in database.
    ///
    /// This method is made public to fill the database for tests, do not use it for
    /// any other purposes.
    #[doc = "hidden"]
    pub async fn store_executed_priority_op(
        &mut self,
        operation: NewExecutedPriorityOperation,
    ) -> QueryResult<()> {
        let start = Instant::now();
        sqlx::query!(
            "INSERT INTO executed_priority_operations (block_number, block_index, operation, from_account, to_account, priority_op_serialid, deadline_block, eth_hash, eth_block, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (priority_op_serialid)
            DO NOTHING",
            operation.block_number,
            operation.block_index,
            operation.operation,
            operation.from_account,
            operation.to_account,
            operation.priority_op_serialid,
            operation.deadline_block,
            operation.eth_hash,
            operation.eth_block,
            operation.created_at,
        )
        .execute(self.0.conn())
        .await?;
        metrics::histogram!(
            "sql.chain.operations.store_executed_priority_op",
            start.elapsed()
        );
        Ok(())
    }

    pub async fn eth_tx_for_withdrawal(
        &mut self,
        withdrawal_hash: &TxHash,
    ) -> QueryResult<Option<H256>> {
        let start = Instant::now();

        let tx_by_hash = OperationsExtSchema(self.0)
            .get_tx_by_hash(withdrawal_hash.as_ref())
            .await?;
        let block_number = if let Some(tx) = tx_by_hash {
            tx.block_number as BlockNumber
        } else {
            return Ok(None);
        };

        let execute_block_operation = self
            .get_aggregated_op_that_affects_block(AggregatedActionType::ExecuteBlocks, block_number)
            .await?;

        let res = if let Some((op_id, _)) = execute_block_operation {
            EthereumSchema(self.0).aggregated_op_final_hash(op_id).await
        } else {
            Ok(None)
        };

        metrics::histogram!(
            "sql.chain.operations.eth_tx_for_withdrawal",
            start.elapsed()
        );
        res
    }

    pub async fn store_aggregated_action(
        &mut self,
        operation: AggregatedOperation,
    ) -> QueryResult<()> {
        let aggregated_action_type = operation.get_action_type();
        let (from_block, to_block) = operation.get_block_range();
        sqlx::query!(
            "INSERT INTO aggregate_operations (action_type, arguments, from_block, to_block)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (id)
            DO NOTHING",
            aggregated_action_type.to_string(),
            serde_json::to_value(operation.clone()).expect("aggregated op serialize fail"),
            i64::from(from_block),
            i64::from(to_block)
        )
        .execute(self.0.conn())
        .await?;
        Ok(())
    }

    pub async fn get_last_affected_block_by_aggregated_action(
        &mut self,
        aggregated_action: AggregatedActionType,
    ) -> QueryResult<BlockNumber> {
        let block_number = sqlx::query!(
            "SELECT max(to_block) from aggregate_operations where action_type = $1",
            aggregated_action.to_string(),
        )
        .fetch_one(self.0.conn())
        .await?
        .max
        .map(|b| b as BlockNumber)
        .unwrap_or_default();
        Ok(block_number)
    }

    pub async fn get_aggregated_op_that_affects_block(
        &mut self,
        aggregated_action: AggregatedActionType,
        block_number: BlockNumber,
    ) -> QueryResult<Option<(i64, AggregatedOperation)>> {
        let aggregated_op = sqlx::query_as!(
            StoredAggregatedOperation,
            "SELECT * FROM aggregate_operations \
            WHERE action_type = $1 and from_block <= $2 and $2 <= to_block",
            aggregated_action.to_string(),
            i64::from(block_number)
        )
        .fetch_optional(self.0.conn())
        .await?
        .map(|op| {
            (
                op.id,
                serde_json::from_value(op.arguments).expect("unparsable aggregated op"),
            )
        });
        Ok(aggregated_op)
    }
}
