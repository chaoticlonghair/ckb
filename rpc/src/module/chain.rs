use crate::error::RPCError;
use ckb_core::cell::{CellProvider, CellStatus};
use ckb_core::transaction::ProposalShortId;
use ckb_shared::shared::Shared;
use ckb_store::ChainStore;
use ckb_traits::ChainProvider;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use jsonrpc_types::{
    BlockNumber, BlockView, Capacity, CellOutPoint, CellOutputWithOutPoint, CellWithStatus,
    EpochExt, EpochNumber, HeaderView, OutPoint, TransactionWithStatus, Unsigned,
};
use numext_fixed_hash::H256;

pub const PAGE_SIZE: u64 = 100;

#[rpc]
pub trait ChainRpc {
    #[rpc(name = "get_block")]
    fn get_block(&self, _hash: H256) -> Result<Option<BlockView>>;

    #[rpc(name = "get_block_by_number")]
    fn get_block_by_number(&self, _number: BlockNumber) -> Result<Option<BlockView>>;

    #[rpc(name = "get_transaction")]
    fn get_transaction(&self, _hash: H256) -> Result<Option<TransactionWithStatus>>;

    #[rpc(name = "get_block_hash")]
    fn get_block_hash(&self, _number: BlockNumber) -> Result<Option<H256>>;

    #[rpc(name = "get_tip_header")]
    fn get_tip_header(&self) -> Result<HeaderView>;

    #[rpc(name = "get_cells_by_lock_hash")]
    fn get_cells_by_lock_hash(
        &self,
        _lock_hash: H256,
        _from: BlockNumber,
        _to: BlockNumber,
    ) -> Result<Vec<CellOutputWithOutPoint>>;

    #[rpc(name = "get_live_cell")]
    fn get_live_cell(&self, _out_point: OutPoint) -> Result<CellWithStatus>;

    #[rpc(name = "get_tip_block_number")]
    fn get_tip_block_number(&self) -> Result<BlockNumber>;

    #[rpc(name = "get_current_epoch")]
    fn get_current_epoch(&self) -> Result<EpochExt>;

    #[rpc(name = "get_epoch_by_number")]
    fn get_epoch_by_number(&self, number: EpochNumber) -> Result<Option<EpochExt>>;
}

pub(crate) struct ChainRpcImpl<CS> {
    pub shared: Shared<CS>,
}

impl<CS: ChainStore + 'static> ChainRpc for ChainRpcImpl<CS> {
    fn get_block(&self, hash: H256) -> Result<Option<BlockView>> {
        Ok(self.shared.block(&hash).as_ref().map(Into::into))
    }

    fn get_block_by_number(&self, number: BlockNumber) -> Result<Option<BlockView>> {
        Ok(self
            .shared
            .block_hash(number.0)
            .and_then(|hash| self.shared.block(&hash).as_ref().map(Into::into)))
    }

    fn get_transaction(&self, hash: H256) -> Result<Option<TransactionWithStatus>> {
        let id = ProposalShortId::from_tx_hash(&hash);

        let tx = {
            let chan_state = self.shared.lock_chain_state();

            let tx_pool = chan_state.tx_pool();
            tx_pool
                .get_tx_from_proposed(&id)
                .map(TransactionWithStatus::with_proposed)
                .or_else(|| {
                    tx_pool
                        .get_tx_without_conflict(&id)
                        .map(TransactionWithStatus::with_pending)
                })
        };

        Ok(tx.or_else(|| {
            self.shared
                .get_transaction(&hash)
                .map(|(tx, block_hash)| TransactionWithStatus::with_committed(tx, block_hash))
        }))
    }

    fn get_block_hash(&self, number: BlockNumber) -> Result<Option<H256>> {
        Ok(self.shared.block_hash(number.0))
    }

    fn get_tip_header(&self) -> Result<HeaderView> {
        Ok(self
            .shared
            .store()
            .get_tip_header()
            .as_ref()
            .map(Into::into)
            .expect("tip header exists"))
    }

    fn get_current_epoch(&self) -> Result<EpochExt> {
        Ok(self
            .shared
            .store()
            .get_current_epoch_ext()
            .map(Into::into)
            .expect("current_epoch exists"))
    }

    fn get_epoch_by_number(&self, number: EpochNumber) -> Result<Option<EpochExt>> {
        Ok(self
            .shared
            .store()
            .get_epoch_index(number.0)
            .and_then(|hash| self.shared.store().get_epoch_ext(&hash).map(Into::into)))
    }

    // TODO: we need to build a proper index instead of scanning every time
    fn get_cells_by_lock_hash(
        &self,
        lock_hash: H256,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<CellOutputWithOutPoint>> {
        let from = from.0;
        let to = to.0;
        if from > to {
            Err(RPCError::custom(
                RPCError::Invalid,
                "from greater than to".to_owned(),
            ))
        } else if to - from > PAGE_SIZE {
            Err(RPCError::custom(
                RPCError::Invalid,
                "too large page size".to_owned(),
            ))
        } else if to == from {
            Ok(Vec::new())
        } else {
            let mut result = Vec::new();

            let mut block_hashes = Vec::with_capacity((to - from + 1) as usize);
            for block_number in from..=to {
                if let Some(block_hash) = self.shared.block_hash(block_number) {
                    block_hashes.push(block_hash);
                } else {
                    break;
                }
            }

            let mut blocks = Vec::new();
            for block_hash in block_hashes.iter() {
                let block = self
                    .shared
                    .block(block_hash)
                    .ok_or_else(Error::internal_error)?;
                blocks.push(block)
            }

            {
                let chain_state = self.shared.lock_chain_state();
                for block in blocks.iter() {
                    let block_hash = block.header().hash();
                    for transaction in block.transactions() {
                        let tx_hash = transaction.hash();
                        for (index, output) in transaction.outputs().iter().enumerate() {
                            let index = index as u32;
                            if chain_state.cell_set().contains_probably(tx_hash, index)
                                && self.shared.has_live_cell(tx_hash, index)
                                && output.lock.hash() == lock_hash
                            {
                                result.push(CellOutputWithOutPoint {
                                    out_point: OutPoint {
                                        cell: Some(CellOutPoint {
                                            tx_hash: tx_hash.to_owned(),
                                            index: Unsigned(index as u64),
                                        }),
                                        block_hash: Some(block_hash.to_owned()),
                                    },
                                    capacity: Capacity(output.capacity),
                                    lock: output.lock.clone().into(),
                                });
                            }
                        }
                    }
                }
                drop(chain_state);
            }

            Ok(result)
        }
    }

    fn get_live_cell(&self, out_point: OutPoint) -> Result<CellWithStatus> {
        let mut cell_status = self
            .shared
            .lock_chain_state()
            .cell(&out_point.clone().into());
        if let CellStatus::Live(ref mut cell_meta) = cell_status {
            if cell_meta.cell_output.is_none() {
                cell_meta.cell_output = Some(
                    self.shared
                        .store()
                        .get_cell_output(&cell_meta.out_point.tx_hash, cell_meta.out_point.index)
                        .expect("live cell must exists"),
                );
            }
        }
        Ok(cell_status.into())
    }

    fn get_tip_block_number(&self) -> Result<BlockNumber> {
        self.get_tip_header().map(|h| h.inner.number)
    }
}
