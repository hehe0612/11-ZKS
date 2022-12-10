use futures::{
    channel::{mpsc, oneshot},
    SinkExt,
};
use std::thread::JoinHandle;
use tokio::runtime::Runtime;
use zksync_core::{
    committer::CommitRequest,
    state_keeper::{
        start_root_hash_calculator, StateKeeperTestkitRequest, ZkSyncStateInitParams,
        ZkSyncStateKeeper,
    },
    tx_event_emitter::ProcessedOperations,
};
use zksync_types::{
    Account, AccountId, Address, DepositOp, FullExitOp, TransferOp, TransferToNewOp, WithdrawOp,
};

use itertools::Itertools;
use zksync_mempool::MempoolBlocksRequest;

pub async fn state_keeper_get_account(
    mut sender: mpsc::Sender<StateKeeperTestkitRequest>,
    address: &Address,
) -> Option<(AccountId, Account)> {
    let resp = oneshot::channel();
    sender
        .send(StateKeeperTestkitRequest::GetAccount(*address, resp.0))
        .await
        .expect("sk request send");
    resp.1.await.expect("sk account resp recv")
}

pub struct StateKeeperChannels {
    pub mempool_receiver: mpsc::Receiver<MempoolBlocksRequest>,
    pub requests: mpsc::Sender<StateKeeperTestkitRequest>,
    pub new_blocks: mpsc::Receiver<CommitRequest>,
    pub queued_txs_events: mpsc::Receiver<ProcessedOperations>,
}

// Thread join handle and stop channel sender.
pub fn spawn_state_keeper(
    fee_account: &Address,
    initial_state: ZkSyncStateInitParams,
) -> (JoinHandle<()>, oneshot::Sender<()>, StateKeeperChannels) {
    let (proposed_blocks_sender, proposed_blocks_receiver) = mpsc::channel(256);
    let (state_keeper_req_sender, state_keeper_req_receiver) = mpsc::channel(256);
    let (mempool_req_sender, mempool_req_receiver) = mpsc::channel(256);
    let (processed_tx_events_sender, processed_tx_events_receiver) = mpsc::channel(256);

    let max_ops_in_block = 1000;
    let ops_chunks = vec![
        TransferToNewOp::CHUNKS,
        TransferOp::CHUNKS,
        DepositOp::CHUNKS,
        FullExitOp::CHUNKS,
        WithdrawOp::CHUNKS,
    ];
    let mut block_chunks_sizes = (0..max_ops_in_block)
        .cartesian_product(ops_chunks)
        .map(|(x, y)| x * y)
        .collect::<Vec<_>>();
    block_chunks_sizes.sort_unstable();
    block_chunks_sizes.dedup();

    let max_miniblock_iterations = *block_chunks_sizes.iter().max().unwrap();
    let (state_keeper, root_hash_calculator) = ZkSyncStateKeeper::new(
        initial_state,
        *fee_account,
        proposed_blocks_sender,
        mempool_req_sender,
        block_chunks_sizes,
        max_miniblock_iterations,
        max_miniblock_iterations,
        processed_tx_events_sender,
    );

    let (stop_state_keeper_sender, stop_state_keeper_receiver) = oneshot::channel::<()>();
    let sk_thread_handle = std::thread::spawn(move || {
        let main_runtime = Runtime::new().expect("main runtime start");
        main_runtime.block_on(async move {
            let state_keeper_task =
                tokio::spawn(state_keeper.run_for_testkit(state_keeper_req_receiver));
            let root_hash_calculator_task = start_root_hash_calculator(root_hash_calculator);
            tokio::select! {
                _ = stop_state_keeper_receiver => {},
                _ = root_hash_calculator_task => {},
                _ = state_keeper_task => {},
            }
        })
    });

    (
        sk_thread_handle,
        stop_state_keeper_sender,
        StateKeeperChannels {
            mempool_receiver: mempool_req_receiver,
            requests: state_keeper_req_sender,
            new_blocks: proposed_blocks_receiver,
            queued_txs_events: processed_tx_events_receiver,
        },
    )
}
