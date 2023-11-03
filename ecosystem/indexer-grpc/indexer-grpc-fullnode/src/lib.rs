// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

use aptos_api::context::Context;
use std::sync::Arc;

pub mod convert;
pub mod counters;
pub mod fullnode_data_service;
pub mod localnet_data_service;
pub mod runtime;
// pub mod runtime_for_table_info;
pub mod stream_coordinator;
pub mod table_info_parser;
// pub mod table_info_parser_multithread;

#[derive(Clone, Debug)]
pub struct ServiceContext {
    pub context: Arc<Context>,
    pub processor_task_count: u16,
    pub processor_batch_size: u16,
    pub output_batch_size: u16,
}

#[cfg(test)]
pub(crate) mod tests;
