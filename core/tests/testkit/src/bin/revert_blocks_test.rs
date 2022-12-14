use web3::transports::Http;

use zksync_testkit::*;
use zksync_testkit::{
    data_restore::verify_restore,
    scenarios::{perform_basic_operations, BlockProcessing},
};

use crate::eth_account::{parse_ether, EthereumAccount};
use crate::external_commands::{deploy_contracts, get_test_accounts, Contracts};
use crate::zksync_account::ZkSyncAccount;
use zksync_crypto::Fr;

/// Executes blocks with some basic operations with new state keeper
/// if block_processing is equal to BlockProcessing::NoVerify this should revert all not verified blocks
async fn execute_blocks_with_new_state_keeper(
    contracts: Contracts,
    block_processing: BlockProcessing,
    fee_account: &ZkSyncAccount,
    genesis_hash: Fr,
) {
    let testkit_config = TestkitConfig::from_env();

    let (sk_thread_handle, stop_state_keeper_sender, sk_channels) =
        spawn_state_keeper(&fee_account.address);

    let fee_account_address = fee_account.address;
    let transport = Http::new(&testkit_config.web3_url).expect("http transport start");
    let (test_accounts_info, commit_account_info) = get_test_accounts();
    let commit_account = EthereumAccount::new(
        commit_account_info.private_key,
        commit_account_info.address,
        transport.clone(),
        contracts.contract,
        testkit_config.chain_id,
        testkit_config.gas_price_factor,
    );
    let eth_accounts = test_accounts_info
        .into_iter()
        .map(|test_eth_account| {
            EthereumAccount::new(
                test_eth_account.private_key,
                test_eth_account.address,
                transport.clone(),
                contracts.contract,
                testkit_config.chain_id,
                testkit_config.gas_price_factor,
            )
        })
        .collect::<Vec<_>>();

    let zksync_accounts = {
        let mut zksync_accounts = Vec::new();
        zksync_accounts.push(fee_account.clone());
        zksync_accounts.extend(eth_accounts.iter().map(|eth_account| {
            let rng_zksync_key = ZkSyncAccount::rand().private_key;
            ZkSyncAccount::new(
                rng_zksync_key,
                0,
                eth_account.address,
                eth_account.private_key,
            )
        }));
        zksync_accounts
    };

    let accounts = AccountSet {
        eth_accounts,
        zksync_accounts,
        fee_account_id: ZKSyncAccountId(0),
    };

    let mut test_setup = TestSetup::new(
        sk_channels,
        accounts,
        &contracts,
        commit_account,
        genesis_hash,
    );

    let deposit_amount = parse_ether("1.0").unwrap();

    let mut executed_blocks = Vec::new();
    let mut tokens = vec![];
    for token in 0..=1 {
        let blocks = perform_basic_operations(
            token,
            &mut test_setup,
            deposit_amount.clone(),
            block_processing,
        )
        .await;
        tokens.push(token);
        executed_blocks.extend(blocks.into_iter());
    }

    if block_processing == BlockProcessing::NoVerify {
        let blocks_committed = test_setup
            .total_blocks_committed()
            .await
            .expect("total_blocks_committed call fails");
        let blocks_verified = test_setup
            .total_blocks_verified()
            .await
            .expect("total_blocks_verified call fails");
        assert_ne!(blocks_committed, blocks_verified, "no blocks to revert");

        let executed_blocks_reverse_order = executed_blocks.into_iter().rev().collect::<Vec<_>>();
        test_setup
            .revert_blocks(&executed_blocks_reverse_order)
            .await
            .expect("revert_blocks call fails");
    } else {
        // Do not restore in reverting state, because there no valid blocks in blockchain
        println!("Start restoring");

        verify_restore(
            &testkit_config.web3_url,
            testkit_config.available_block_chunk_sizes.clone(),
            &contracts,
            fee_account_address,
            test_setup.get_accounts_state().await,
            tokens,
            test_setup.last_committed_block.new_root_hash,
        )
        .await;
    }

    stop_state_keeper_sender.send(()).expect("sk stop send");
    sk_thread_handle.join().expect("sk thread join");
}

async fn revert_blocks_test() {
    let fee_account = ZkSyncAccount::rand();
    let genesis_hash = genesis_state(&fee_account.address).tree.root_hash();
    println!("deploying contracts");
    let contracts = deploy_contracts(false, genesis_hash);
    println!("contracts deployed");

    execute_blocks_with_new_state_keeper(
        contracts.clone(),
        BlockProcessing::NoVerify,
        &fee_account,
        genesis_hash,
    )
    .await;
    println!("some blocks are committed and reverted\n\n");

    execute_blocks_with_new_state_keeper(
        contracts,
        BlockProcessing::CommitAndVerify,
        &fee_account,
        genesis_hash,
    )
    .await;
}

#[tokio::main]
async fn main() {
    revert_blocks_test().await;
}
