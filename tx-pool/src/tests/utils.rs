use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

use ckb_app_config::{BlockAssemblerConfig, NetworkConfig, TxPoolConfig};
use ckb_async_runtime::{new_global_runtime, Handle};
use ckb_chain_spec::{build_genesis_epoch_ext, consensus::Consensus};
use ckb_channel::Receiver;
use ckb_dao_utils::genesis_dao_data;
use ckb_network::{DefaultExitHandler, NetworkController, NetworkService, NetworkState, PeerIndex};
use ckb_proposal_table::ProposalView;
use ckb_snapshot::Snapshot;
use ckb_stop_handler::StopHandler;
use ckb_store::ChainStore;
use ckb_test_chain_utils::{always_success_cell, MockStore};
use ckb_types::{
    core::{
        capacity_bytes, hardfork::HardForkSwitch, tx_pool::Reject, BlockNumber, BlockView,
        Capacity, DepType, EpochExt, EpochNumber, EpochNumberWithFraction, FeeRate, ScriptHashType,
        TransactionView,
    },
    packed,
    prelude::*,
    utilities::{difficulty_to_compact, DIFF_TWO},
    U256,
};
use ckb_verification::cache::init_cache;
use faketime::unix_time_as_millis;

use crate::{TokioRwLock, TxEntry, TxPool, TxPoolController, TxPoolServiceBuilder};

pub(crate) const PRIMARY_EPOCH_REWARD: Capacity = capacity_bytes!(1_000_000);
pub(crate) const EPOCH_LENGTH: BlockNumber = 10;
pub(crate) const EPOCH_WHEN_HARDFORK: EpochNumber = 2;
pub(crate) const DEFAULT_ORPHAN_RATE_TARGET: (u32, u32) = (1, 40);
pub(crate) const EPOCH_DURATION: u64 = 60;

const NETWORK_NAME: &str = "TxPool FSM Network";
const NETWORK_VERSION: &str = "TxPool FSM Network v0.1.0";
const BINARY_VERSION: &str = "CKB TxPool Finite State Machine";

#[derive(Clone)]
pub(crate) struct ScriptInfo {
    data: packed::Bytes,
    cell_dep: packed::CellDep,
    _data_hash: packed::Byte32,
    type_hash: packed::Byte32,
}

pub(crate) struct MockChain {
    consensus: Arc<Consensus>,
    _default_script_info: ScriptInfo,
    mock_store: MockStore,
    current_snapshot: Arc<Snapshot>,
    _handle: Handle,
    _stop_handler: StopHandler<()>,
    tx_pool_controller: TxPoolController,
    _network_controller: NetworkController,
    _tx_relay_receiver: Receiver<(Option<PeerIndex>, bool, packed::Byte32)>,
}

impl ScriptInfo {
    pub(crate) fn type_hash(&self) -> packed::Byte32 {
        self.type_hash.clone()
    }
}

impl MockChain {
    pub(crate) fn new() -> Self {
        let (consensus, default_script_info) = Self::build_consensus();
        let consensus = Arc::new(consensus);
        let mock_store = MockStore::default();
        mock_store.store().init(&consensus).unwrap();
        let current_snapshot = {
            let store = mock_store.store().get_snapshot();
            let proposals = ProposalView::default();
            let tip_header = consensus.genesis_block().header();
            let total_difficulty = consensus.genesis_block().difficulty();
            let genesis_epoch_ext = consensus.genesis_epoch_ext().to_owned();
            let snapshot = Snapshot::new(
                tip_header,
                total_difficulty,
                genesis_epoch_ext,
                store,
                proposals,
                Arc::clone(&consensus),
            );
            Arc::new(snapshot)
        };
        let (handle, stop_handler) = new_global_runtime();
        let network_controller = Self::dummy_network(&handle);
        let (tx_pool_controller, tx_relay_receiver) = {
            let mut tx_pool_config = TxPoolConfig::default();
            tx_pool_config.min_fee_rate = FeeRate(0);
            let block_assembler_config = BlockAssemblerConfig {
                code_hash: default_script_info.type_hash().unpack(),
                args: Default::default(),
                hash_type: ScriptHashType::Type.into(),
                message: Default::default(),
                use_binary_version_as_message_prefix: true,
                binary_version: BINARY_VERSION.to_owned(),
            };
            let txs_verify_cache = {
                let cache = init_cache();
                Arc::new(TokioRwLock::new(cache))
            };
            let (tx_relay_sender, tx_relay_receiver) = ckb_channel::unbounded();
            let (mut tx_pool_builder, tx_pool_controller) = TxPoolServiceBuilder::new(
                tx_pool_config,
                current_snapshot.clone(),
                Some(block_assembler_config),
                txs_verify_cache,
                &handle,
                tx_relay_sender,
            );
            Self::register_tx_pool_callback(&mut tx_pool_builder);
            tx_pool_builder.start(network_controller.clone());
            (tx_pool_controller, tx_relay_receiver)
        };

        assert!(tx_pool_controller.service_started());

        Self {
            consensus,
            _default_script_info: default_script_info,
            mock_store,
            current_snapshot,
            _handle: handle,
            _stop_handler: stop_handler,
            tx_pool_controller,
            _network_controller: network_controller,
            _tx_relay_receiver: tx_relay_receiver,
        }
    }

    pub(crate) fn consensus(&self) -> Arc<Consensus> {
        Arc::clone(&self.consensus)
    }

    pub(crate) fn current_snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.current_snapshot)
    }

    pub(crate) fn next_epoch_ext(&self) -> EpochExt {
        self.consensus
            .next_epoch_ext(
                self.current_snapshot().tip_header(),
                &self.mock_store.store().as_data_provider(),
            )
            .unwrap()
            .epoch()
    }

    pub(crate) fn insert_block(&mut self, block: &BlockView) {
        let snapshot = self.current_snapshot();
        let total_difficulty = snapshot.total_difficulty() + block.difficulty();
        let proposals = snapshot.proposals().to_owned();
        let next_epoch_ext = self.next_epoch_ext();
        let tip_header = block.header();
        self.mock_store.insert_block(block, &next_epoch_ext);
        let store = self.mock_store.store().get_snapshot();
        self.current_snapshot = {
            let snapshot = Snapshot::new(
                tip_header,
                total_difficulty,
                next_epoch_ext,
                store,
                proposals,
                self.consensus(),
            );
            Arc::new(snapshot)
        };
    }

    pub(crate) fn tick(&mut self) {
        let snapshot = self.current_snapshot();
        let block_template = self
            .tx_pool_controller()
            .get_block_template(None, None, None, snapshot)
            .unwrap()
            .unwrap();
        let block: packed::Block = block_template.into();
        let block_view = block.into_view();

        let result = self.tx_pool_controller().update_tx_pool_for_reorg(
            VecDeque::default(),
            vec![block_view.clone()].into_iter().collect(),
            HashSet::default(),
            self.current_snapshot(),
        );
        assert!(result.is_ok());
        self.insert_block(&block_view);
    }

    pub(crate) fn tx_pool_controller(&self) -> &TxPoolController {
        &self.tx_pool_controller
    }

    fn build_consensus() -> (Consensus, ScriptInfo) {
        let (_, script_data, _) = always_success_cell();
        let script_data_capacity = Capacity::bytes(script_data.len()).unwrap();
        let script_data_hash = packed::CellOutput::calc_data_hash(script_data);
        let (default_script_info, genesis_block) = {
            // Deploy always success script in genesis cellbase.
            let (tx0, script0) = {
                let output = packed::CellOutput::new_builder()
                    .build_exact_capacity(script_data_capacity)
                    .unwrap();
                let script0 = packed::Script::new_builder()
                    .hash_type(ScriptHashType::Data.into())
                    .code_hash(script_data_hash.clone())
                    .build();
                let tx0 = TransactionView::new_advanced_builder()
                    .input(packed::CellInput::new_cellbase_input(0))
                    .output(output)
                    .output_data(script_data.pack())
                    .witness(script0.clone().into_witness())
                    .build();
                (tx0, script0)
            };
            // Deploy always success script again with type script.
            let tx1 = {
                let output = packed::CellOutput::new_builder()
                    .type_(Some(script0.clone()).pack())
                    .build_exact_capacity(script_data_capacity)
                    .unwrap();
                let script0_cell_dep = {
                    let script0_cell_op = packed::OutPoint::new(tx0.hash(), 0);
                    packed::CellDep::new_builder()
                        .out_point(script0_cell_op)
                        .dep_type(DepType::Code.into())
                        .build()
                };
                let script1 = {
                    let type_hash = script0.calc_script_hash();
                    packed::Script::new_builder()
                        .code_hash(type_hash)
                        .hash_type(ScriptHashType::Type.into())
                        .build()
                };
                TransactionView::new_advanced_builder()
                    .cell_dep(script0_cell_dep.clone())
                    .input(packed::CellInput::new_cellbase_input(0))
                    .output(output)
                    .output_data(script_data.pack())
                    .witness(script1.into_witness())
                    .build()
            };
            let script1_cell_dep = {
                let script1_cell_op = packed::OutPoint::new(tx1.hash(), 0);
                packed::CellDep::new_builder()
                    .out_point(script1_cell_op)
                    .dep_type(DepType::Code.into())
                    .build()
            };
            let script0_hash = script0.calc_script_hash();
            let default_script_info = ScriptInfo {
                data: script_data.pack(),
                cell_dep: script1_cell_dep,
                _data_hash: script_data_hash,
                type_hash: script0_hash,
            };
            let dao = genesis_dao_data(vec![&tx0, &tx1]).unwrap();
            let genesis_block = packed::Block::new_advanced_builder()
                .timestamp(unix_time_as_millis().pack())
                .dao(dao)
                .compact_target(difficulty_to_compact(U256::from(100u64)).pack())
                .transaction(tx0)
                .transaction(tx1)
                .build();
            (default_script_info, genesis_block)
        };
        let genesis_epoch_ext = build_genesis_epoch_ext(
            PRIMARY_EPOCH_REWARD,
            DIFF_TWO,
            EPOCH_LENGTH,
            EPOCH_DURATION,
            DEFAULT_ORPHAN_RATE_TARGET,
        );
        let hardfork_switch = HardForkSwitch::new_builder()
            .rfc_0028(EPOCH_WHEN_HARDFORK)
            .rfc_0029(EPOCH_WHEN_HARDFORK)
            .rfc_0030(EPOCH_WHEN_HARDFORK)
            .rfc_0031(EPOCH_WHEN_HARDFORK)
            .rfc_0032(EPOCH_WHEN_HARDFORK)
            .rfc_0036(EPOCH_WHEN_HARDFORK)
            .rfc_0038(EPOCH_WHEN_HARDFORK)
            .build()
            .unwrap();
        let consensus = Consensus {
            permanent_difficulty_in_dummy: true,
            hardfork_switch,
            genesis_block,
            cellbase_maturity: EpochNumberWithFraction::new(0, 0, 1),
            genesis_epoch_ext,
            ..Default::default()
        };
        (consensus, default_script_info)
    }

    fn dummy_network(handle: &Handle) -> NetworkController {
        let tmp_dir = tempfile::Builder::new().tempdir().unwrap();
        let config = NetworkConfig {
            max_peers: 19,
            max_outbound_peers: 5,
            path: tmp_dir.path().to_path_buf(),
            ping_interval_secs: 15,
            ping_timeout_secs: 20,
            connect_outbound_interval_secs: 1,
            discovery_local_address: true,
            bootnode_mode: true,
            reuse_port_on_linux: true,
            ..Default::default()
        };

        let network_state = Arc::new(NetworkState::from_config(config).unwrap());
        NetworkService::new(
            network_state,
            vec![],
            vec![],
            NETWORK_NAME.to_owned(),
            NETWORK_VERSION.to_owned(),
            DefaultExitHandler::default(),
        )
        .start(handle)
        .unwrap()
    }

    fn register_tx_pool_callback(tx_pool_builder: &mut TxPoolServiceBuilder) {
        tx_pool_builder.register_pending(Box::new(move |tx_pool: &mut TxPool, entry: &TxEntry| {
            tx_pool.update_statics_for_add_tx(entry.size, entry.cycles);
        }));

        tx_pool_builder.register_proposed(Box::new(
            move |tx_pool: &mut TxPool, entry: &TxEntry, new: bool| {
                if new {
                    tx_pool.update_statics_for_add_tx(entry.size, entry.cycles);
                }
            },
        ));

        tx_pool_builder.register_committed(Box::new(
            move |tx_pool: &mut TxPool, entry: &TxEntry| {
                tx_pool.update_statics_for_remove_tx(entry.size, entry.cycles);
            },
        ));

        tx_pool_builder.register_reject(Box::new(
            move |tx_pool: &mut TxPool, entry: &TxEntry, reject: Reject| {
                tx_pool.update_statics_for_remove_tx(entry.size, entry.cycles);
                let tx_hash = entry.transaction().hash();
                if matches!(reject, Reject::Resolve(..)) {
                    if let Some(ref mut recent_reject) = tx_pool.recent_reject {
                        let _ = recent_reject.put(&tx_hash, reject.clone());
                    }
                }
            },
        ));
    }
}
