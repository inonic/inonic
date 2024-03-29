// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::coordinator::{
    ConsensusToCoordinatorMessage, ExecutorProxy, ExecutorProxyTrait, SyncCoordinator,
};
use config::config::NodeConfig;
use failure::prelude::*;
use futures::{
    channel::{mpsc, oneshot},
    future::{Future, FutureExt, TryFutureExt},
    SinkExt,
};
use network::validator_network::{StateSynchronizerEvents, StateSynchronizerSender};
use nextgen_crypto::ed25519::*;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};
use types::ledger_info::LedgerInfoWithSignatures;

pub struct StateSynchronizer {
    _runtime: Runtime,
    coordinator_sender: mpsc::UnboundedSender<ConsensusToCoordinatorMessage>,
}

impl StateSynchronizer {
    /// Setup state synchronizer. spawns coordinator and downloader routines on executor
    pub fn bootstrap(
        network_sender: StateSynchronizerSender,
        network_events: StateSynchronizerEvents,
        config: &NodeConfig,
    ) -> Self {
        let executor_proxy = ExecutorProxy::new(config);
        Self::bootstrap_with_executor_proxy(network_sender, network_events, config, executor_proxy)
    }

    pub fn bootstrap_with_executor_proxy<E: ExecutorProxyTrait + 'static>(
        network_sender: StateSynchronizerSender,
        network_events: StateSynchronizerEvents,
        config: &NodeConfig,
        executor_proxy: E,
    ) -> Self {
        let runtime = Builder::new()
            .name_prefix("state-sync-")
            .build()
            .expect("[state synchronizer] failed to create runtime");
        let executor = runtime.executor();

        let (coordinator_sender, coordinator_receiver) = mpsc::unbounded();

        let coordinator = SyncCoordinator::new(
            network_sender,
            network_events,
            coordinator_receiver,
            config,
            executor_proxy,
        );
        executor.spawn(coordinator.start().boxed().unit_error().compat());

        Self {
            _runtime: runtime,
            coordinator_sender,
        }
    }

    pub fn create_client(&self) -> Arc<StateSyncClient> {
        Arc::new(StateSyncClient {
            coordinator_sender: self.coordinator_sender.clone(),
        })
    }
}

pub struct StateSyncClient {
    coordinator_sender: mpsc::UnboundedSender<ConsensusToCoordinatorMessage>,
}

impl StateSyncClient {
    /// Sync validator's state up to given `version`
    pub fn sync_to(
        &self,
        target: LedgerInfoWithSignatures<Ed25519Signature>,
    ) -> impl Future<Output = Result<bool>> {
        let mut sender = self.coordinator_sender.clone();
        let (cb_sender, cb_receiver) = oneshot::channel();
        async move {
            sender
                .send(ConsensusToCoordinatorMessage::Requested(target, cb_sender))
                .await?;
            let sync_status = cb_receiver.await?;
            Ok(sync_status)
        }
    }

    /// Notifies state synchronizer about new version
    pub fn commit(&self, version: u64) -> impl Future<Output = Result<()>> {
        let mut sender = self.coordinator_sender.clone();
        async move {
            sender
                .send(ConsensusToCoordinatorMessage::Commit(version))
                .await?;
            Ok(())
        }
    }
}
