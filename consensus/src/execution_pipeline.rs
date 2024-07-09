// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

use crate::{
    block_preparer::BlockPreparer,
    monitor,
    state_computer::{PipelineExecutionResult, StateComputeResultFut},
};
use aptos_consensus_types::{block::Block, pipelined_block::OrderedBlockWindow};
use aptos_crypto::HashValue;
use aptos_executor_types::{
    state_checkpoint_output::StateCheckpointOutput, BlockExecutorTrait, ExecutorError,
    ExecutorResult,
};
use aptos_experimental_runtimes::thread_manager::optimal_min_len;
use aptos_logger::{debug, error, info};
use aptos_mempool::counters;
use aptos_storage_interface::{
    state_view::{DbStateView, LatestDbStateCheckpointView},
    DbReader,
};
use aptos_types::{
    account_config::AccountResource,
    block_executor::{config::BlockExecutorConfigFromOnchain, partitioner::ExecutableBlock},
    block_metadata_ext::BlockMetadataExt,
    state_store::MoveResourceExt,
    transaction::{
        signature_verified_transaction::SignatureVerifiedTransaction, SignedTransaction,
    },
};
use fail::fail_point;
use move_core_types::account_address::AccountAddress;
use once_cell::sync::Lazy;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use std::{sync::Arc, time::Instant};
use tokio::sync::{mpsc, oneshot};

pub static SIG_VERIFY_POOL: Lazy<Arc<rayon::ThreadPool>> = Lazy::new(|| {
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(8) // More than 8 threads doesn't seem to help much
            .thread_name(|index| format!("signature-checker-{}", index))
            .build()
            .unwrap(),
    )
});

pub struct ExecutionPipeline {
    prepare_block_tx: mpsc::UnboundedSender<PrepareBlockCommand>,
}

impl ExecutionPipeline {
    pub fn spawn(
        db: Arc<dyn DbReader>,
        executor: Arc<dyn BlockExecutorTrait>,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let (prepare_block_tx, prepare_block_rx) = mpsc::unbounded_channel();
        let (execute_block_tx, execute_block_rx) = mpsc::unbounded_channel();
        let (ledger_apply_tx, ledger_apply_rx) = mpsc::unbounded_channel();
        runtime.spawn(Self::prepare_block_stage(
            prepare_block_rx,
            execute_block_tx,
            db,
        ));
        runtime.spawn(Self::execute_stage(
            execute_block_rx,
            ledger_apply_tx,
            executor.clone(),
        ));
        runtime.spawn(Self::ledger_apply_stage(ledger_apply_rx, executor));
        Self { prepare_block_tx }
    }

    pub async fn queue(
        &self,
        block: Block,
        block_window: OrderedBlockWindow,
        metadata: BlockMetadataExt,
        parent_block_id: HashValue,
        txn_generator: BlockPreparer,
        block_executor_onchain_config: BlockExecutorConfigFromOnchain,
    ) -> StateComputeResultFut {
        let (result_tx, result_rx) = oneshot::channel();
        let block_id = block.id();
        self.prepare_block_tx
            .send(PrepareBlockCommand {
                block,
                block_window,
                metadata,
                block_executor_onchain_config,
                parent_block_id,
                block_preparer: txn_generator,
                result_tx,
            })
            .expect("Failed to send block to execution pipeline.");

        Box::pin(async move {
            result_rx
                .await
                .map_err(|err| ExecutorError::InternalError {
                    error: format!(
                        "Failed to receive execution result for block {}: {:?}.",
                        block_id, err
                    ),
                })?
        })
    }

    /// returns account's sequence number from storage
    fn get_account_sequence_number(
        state_view: &DbStateView,
        address: AccountAddress,
    ) -> anyhow::Result<u64> {
        fail_point!("vm_validator::get_account_sequence_number", |_| {
            Err(anyhow::anyhow!(
                "Injected error in get_account_sequence_number"
            ))
        });

        match AccountResource::fetch_move_resource(state_view, &address)? {
            Some(account_resource) => Ok(account_resource.sequence_number()),
            None => Ok(0),
        }
    }

    // TODO: basically copy-paste mempool vm validator
    async fn prepare_block(
        execute_block_tx: mpsc::UnboundedSender<ExecuteBlockCommand>,
        command: PrepareBlockCommand,
        db: Arc<dyn DbReader>,
    ) {
        let PrepareBlockCommand {
            block,
            block_window,
            metadata,
            block_executor_onchain_config,
            parent_block_id,
            block_preparer,
            result_tx,
        } = command;

        debug!("prepare_block received block {}.", block.id());
        let input_txns = block_preparer.prepare_block(&block, &block_window).await;
        if let Err(e) = input_txns {
            result_tx.send(Err(e)).unwrap_or_else(|err| {
                error!(
                    block_id = block.id(),
                    "Failed to send back execution result for block {}: {:?}.",
                    block.id(),
                    err,
                );
            });
            return;
        }
        let validator_txns = block.validator_txns().cloned().unwrap_or_default();
        let input_txns = input_txns.unwrap();
        tokio::task::spawn_blocking(move || {
            let txns_to_execute =
                Block::combine_to_input_transactions(validator_txns, input_txns.clone(), metadata);

            // // TODO: basically copy-paste from mempool process_incoming_transactions
            // let start_storage_read = Instant::now();
            // let state_view = db
            //     .latest_state_checkpoint_view()
            //     .expect("Failed to get latest state checkpoint view.");
            // let mut num_filtered: u64 = 0;
            // let txns_to_execute = SIG_VERIFY_POOL.install(|| {
            //     txns_to_execute
            //         .into_par_iter()
            //         .map(|t| match t.try_as_signed_user_txn() {
            //             Some(signed_txn) => {
            //                 match Self::get_account_sequence_number(
            //                     &state_view,
            //                     signed_txn.sender(),
            //                 ) {
            //                     Ok(sequence_number) => {
            //                         if signed_txn.sequence_number() >= sequence_number {
            //                             Some(t)
            //                         } else {
            //                             // Sequence number too old
            //                             num_filtered += 1;
            //                             None
            //                         }
            //                     },
            //                     Err(e) => {
            //                         info!("Failed to get sequence number: {:?}", e);
            //                         num_filtered += 1;
            //                         None
            //                     },
            //                 }
            //             },
            //             None => Some(t),
            //         })
            //         .collect::<Vec<_>>()
            // });
            // // Track latency for storage read fetching sequence number
            // let storage_read_latency = start_storage_read.elapsed();
            // // TODO: convert into proper stats
            // info!(
            //     "txns filtered by sequence number: {}/{}, in {} ms",
            //     num_filtered,
            //     txns_to_execute.len(),
            //     storage_read_latency.as_millis()
            // );

            let txns_to_execute_len = txns_to_execute.len();
            let start_storage_read = Instant::now();
            let state_view = db
                .latest_state_checkpoint_view()
                .expect("Failed to get latest state checkpoint view.");
            // TODO: this is a hack, just say an old transaction is invalid
            let sig_verified_txns: Vec<SignatureVerifiedTransaction> =
                SIG_VERIFY_POOL.install(|| {
                    let num_txns = txns_to_execute.len();
                    txns_to_execute
                        .into_par_iter()
                        .with_min_len(optimal_min_len(num_txns, 32))
                        .map(|t| match t.try_as_signed_user_txn() {
                            Some(signed_txn) => match Self::get_account_sequence_number(
                                &state_view,
                                signed_txn.sender(),
                            ) {
                                Ok(sequence_number) => {
                                    if signed_txn.sequence_number() >= sequence_number {
                                        t.into()
                                    } else {
                                        SignatureVerifiedTransaction::Invalid(t)
                                    }
                                },
                                Err(e) => {
                                    info!("Failed to get sequence number: {:?}", e);
                                    SignatureVerifiedTransaction::Invalid(t)
                                },
                            },
                            None => t.into(),
                        })
                        .collect::<Vec<_>>()
                });
            let storage_read_latency = start_storage_read.elapsed();

            let num_old = sig_verified_txns
                .iter()
                .filter(|t| matches!(t, SignatureVerifiedTransaction::Invalid(_)))
                .count();
            info!(
                "txns filtered by sequence number: {}/{}, in {} ms",
                num_old,
                txns_to_execute_len,
                storage_read_latency.as_millis()
            );
            execute_block_tx
                .send(ExecuteBlockCommand {
                    input_txns,
                    block: (block.id(), sig_verified_txns).into(),
                    parent_block_id,
                    block_executor_onchain_config,
                    result_tx,
                })
                .expect("Failed to send block to execution pipeline.");
        })
        .await
        .expect("Failed to spawn_blocking.");
    }

    async fn prepare_block_stage(
        mut prepare_block_rx: mpsc::UnboundedReceiver<PrepareBlockCommand>,
        execute_block_tx: mpsc::UnboundedSender<ExecuteBlockCommand>,
        db: Arc<dyn DbReader>,
    ) {
        while let Some(command) = prepare_block_rx.recv().await {
            monitor!(
                "prepare_block",
                Self::prepare_block(execute_block_tx.clone(), command, db.clone()).await
            );
        }
        debug!("prepare_block_stage quitting.");
    }

    async fn execute_stage(
        mut block_rx: mpsc::UnboundedReceiver<ExecuteBlockCommand>,
        ledger_apply_tx: mpsc::UnboundedSender<LedgerApplyCommand>,
        executor: Arc<dyn BlockExecutorTrait>,
    ) {
        while let Some(ExecuteBlockCommand {
            input_txns,
            block,
            parent_block_id,
            block_executor_onchain_config,
            result_tx,
        }) = block_rx.recv().await
        {
            let block_id = block.block_id;
            debug!("execute_stage received block {}.", block_id);
            let executor = executor.clone();
            let state_checkpoint_output = monitor!(
                "execute_block",
                tokio::task::spawn_blocking(move || {
                    fail_point!("consensus::compute", |_| {
                        Err(ExecutorError::InternalError {
                            error: "Injected error in compute".into(),
                        })
                    });
                    executor.execute_and_state_checkpoint(
                        block,
                        parent_block_id,
                        block_executor_onchain_config,
                    )
                })
                .await
            )
            .expect("Failed to spawn_blocking.");

            ledger_apply_tx
                .send(LedgerApplyCommand {
                    input_txns,
                    block_id,
                    parent_block_id,
                    state_checkpoint_output,
                    result_tx,
                })
                .expect("Failed to send block to ledger_apply stage.");
        }
        debug!("execute_stage quitting.");
    }

    async fn ledger_apply_stage(
        mut block_rx: mpsc::UnboundedReceiver<LedgerApplyCommand>,
        executor: Arc<dyn BlockExecutorTrait>,
    ) {
        while let Some(LedgerApplyCommand {
            input_txns,
            block_id,
            parent_block_id,
            state_checkpoint_output,
            result_tx,
        }) = block_rx.recv().await
        {
            debug!("ledger_apply stage received block {}.", block_id);
            let res = async {
                let executor = executor.clone();
                monitor!(
                    "ledger_apply",
                    tokio::task::spawn_blocking(move || {
                        executor.ledger_update(block_id, parent_block_id, state_checkpoint_output?)
                    })
                )
                .await
                .expect("Failed to spawn_blocking().")
            }
            .await;
            let pipe_line_res = res.map(|output| PipelineExecutionResult::new(input_txns, output));
            result_tx.send(pipe_line_res).unwrap_or_else(|err| {
                error!(
                    block_id = block_id,
                    "Failed to send back execution result for block {}: {:?}", block_id, err,
                );
            });
        }
        debug!("ledger_apply stage quitting.");
    }
}

struct PrepareBlockCommand {
    block: Block,
    block_window: OrderedBlockWindow,
    metadata: BlockMetadataExt,
    block_executor_onchain_config: BlockExecutorConfigFromOnchain,
    // The parent block id.
    parent_block_id: HashValue,
    block_preparer: BlockPreparer,
    result_tx: oneshot::Sender<ExecutorResult<PipelineExecutionResult>>,
}

struct ExecuteBlockCommand {
    input_txns: Vec<SignedTransaction>,
    block: ExecutableBlock,
    parent_block_id: HashValue,
    block_executor_onchain_config: BlockExecutorConfigFromOnchain,
    result_tx: oneshot::Sender<ExecutorResult<PipelineExecutionResult>>,
}

struct LedgerApplyCommand {
    input_txns: Vec<SignedTransaction>,
    block_id: HashValue,
    parent_block_id: HashValue,
    state_checkpoint_output: ExecutorResult<StateCheckpointOutput>,
    result_tx: oneshot::Sender<ExecutorResult<PipelineExecutionResult>>,
}
