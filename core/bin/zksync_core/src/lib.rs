use zksync_types::{tokens::get_genesis_token_list, Token, TokenId};

use crate::{
    block_proposer::run_block_proposer_task,
    committer::run_committer,
    eth_watch::start_eth_watch,
    mempool::run_mempool_task,
    private_api::start_private_core_api,
    state_keeper::{start_state_keeper, ZkSyncStateInitParams, ZkSyncStateKeeper},
};
use futures::{channel::mpsc, future};
use tokio::task::JoinHandle;
use zksync_config::{ApiServerOptions, ConfigurationOptions};
use zksync_storage::ConnectionPool;

const DEFAULT_CHANNEL_CAPACITY: usize = 32_768;

pub mod block_proposer;
pub mod committer;
pub mod eth_watch;
pub mod mempool;
pub mod private_api;
pub mod state_keeper;

/// Waits for *any* of the tokio tasks to be finished.
/// Since the main tokio tasks are used as actors which should live as long
/// as application runs, any possible outcome (either `Ok` or `Err`) is considered
/// as a reason to stop the server completely.
pub async fn wait_for_tasks(task_futures: Vec<JoinHandle<()>>) {
    match future::select_all(task_futures).await {
        (Ok(_), _, _) => {
            panic!("One of the actors finished its run, while it wasn't expected to do it");
        }
        (Err(error), _, _) => {
            log::warn!("One of the tokio actors unexpectedly finished, shutting down");
            if error.is_panic() {
                // Resume the panic on the main task
                std::panic::resume_unwind(error.into_panic());
            }
        }
    }
}

/// Inserts the initial information about zkSync tokens into the database.
pub async fn genesis_init() {
    let pool = ConnectionPool::new(Some(1));
    let config_options = ConfigurationOptions::from_env();

    log::info!("Generating genesis block.");
    ZkSyncStateKeeper::create_genesis_block(pool.clone(), &config_options.operator_fee_eth_addr)
        .await;
    log::info!("Adding initial tokens to db");
    let genesis_tokens =
        get_genesis_token_list(&config_options.eth_network).expect("Initial token list not found");
    for (id, token) in (1..).zip(genesis_tokens) {
        log::info!(
            "Adding token: {}, id:{}, address: {}, decimals: {}",
            token.symbol,
            id,
            token.address,
            token.decimals
        );
        pool.access_storage()
            .await
            .expect("failed to access db")
            .tokens_schema()
            .store_token(Token {
                id: id as TokenId,
                symbol: token.symbol,
                address: token.address[2..]
                    .parse()
                    .expect("failed to parse token address"),
                decimals: token.decimals,
            })
            .await
            .expect("failed to store token");
    }
}

/// Starts the core application, which has the following sub-modules:
///
/// - Ethereum Watcher, module to monitor on-chain operations.
/// - zkSync state keeper, module to execute and seal blocks.
/// - mempool, module to organize incoming transactions.
/// - block proposer, module to create block proposals for state keeper.
/// - committer, module to store pending and completed blocks into the database.
/// - private Core API server.
pub async fn run_core(
    connection_pool: ConnectionPool,
    panic_notify: mpsc::Sender<bool>,
) -> anyhow::Result<Vec<JoinHandle<()>>> {
    let config_opts = ConfigurationOptions::from_env();
    let api_server_options = ApiServerOptions::from_env();

    let (proposed_blocks_sender, proposed_blocks_receiver) =
        mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (state_keeper_req_sender, state_keeper_req_receiver) =
        mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (eth_watch_req_sender, eth_watch_req_receiver) = mpsc::channel(DEFAULT_CHANNEL_CAPACITY);
    let (mempool_request_sender, mempool_request_receiver) =
        mpsc::channel(DEFAULT_CHANNEL_CAPACITY);

    // Start Ethereum Watcher.
    let eth_watch_task = start_eth_watch(
        config_opts.clone(),
        eth_watch_req_sender.clone(),
        eth_watch_req_receiver,
        connection_pool.clone(),
    );

    // Insert pending withdrawals into database (if required)
    let mut storage_processor = connection_pool.access_storage().await?;

    // Start State Keeper.
    let state_keeper_init = ZkSyncStateInitParams::restore_from_db(&mut storage_processor).await?;
    let pending_block = state_keeper_init
        .get_pending_block(&mut storage_processor)
        .await;

    let state_keeper = ZkSyncStateKeeper::new(
        state_keeper_init,
        config_opts.operator_fee_eth_addr,
        state_keeper_req_receiver,
        proposed_blocks_sender,
        config_opts.available_block_chunk_sizes.clone(),
        config_opts.miniblock_timings.max_miniblock_iterations,
        config_opts.miniblock_timings.fast_miniblock_iterations,
        config_opts.max_number_of_withdrawals_per_block,
    );
    let state_keeper_task = start_state_keeper(state_keeper, pending_block);

    // Start committer.
    let committer_task = run_committer(
        proposed_blocks_receiver,
        mempool_request_sender.clone(),
        connection_pool.clone(),
    );

    // Start mempool.
    let mempool_task = run_mempool_task(
        connection_pool.clone(),
        mempool_request_receiver,
        eth_watch_req_sender.clone(),
        &config_opts,
    );

    // Start block proposer.
    let proposer_task = run_block_proposer_task(
        &config_opts,
        mempool_request_sender.clone(),
        state_keeper_req_sender.clone(),
    );

    // Start private API.
    start_private_core_api(
        panic_notify.clone(),
        mempool_request_sender,
        eth_watch_req_sender,
        api_server_options,
    );

    let task_futures = vec![
        eth_watch_task,
        state_keeper_task,
        committer_task,
        mempool_task,
        proposer_task,
    ];

    Ok(task_futures)
}
