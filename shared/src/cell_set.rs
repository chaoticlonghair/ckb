use bloom_filters::{
    BloomFilter, CountingBloomFilter, DefaultBuildHashKernels, RemovableBloomFilter,
};
use ckb_core::block::Block;
use ckb_core::transaction::{CellKey, OutPoint};
use ckb_core::transaction_meta::TransactionMeta;
use ckb_util::{FnvHashMap, FnvHashSet};
use numext_fixed_hash::H256;
use serde_derive::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;

#[derive(Default, Clone, Deserialize, Serialize)]
pub struct CellSetDiff {
    pub old_inputs: FnvHashSet<OutPoint>,
    pub old_outputs: FnvHashMap<H256, usize>,
    pub new_inputs: FnvHashSet<OutPoint>,
    pub new_outputs: FnvHashMap<H256, (u64, u64, bool, usize)>,
}

impl CellSetDiff {
    pub fn push_new(&mut self, block: &Block) {
        for tx in block.transactions() {
            let input_iter = tx.input_pts_iter();
            let tx_hash = tx.hash();
            let output_len = tx.outputs().len();
            self.new_inputs.extend(input_iter.cloned());
            self.new_outputs.insert(
                tx_hash.to_owned(),
                (
                    block.header().number(),
                    block.header().epoch(),
                    tx.is_cellbase(),
                    output_len,
                ),
            );
        }
    }

    pub fn push_old(&mut self, block: &Block) {
        for tx in block.transactions() {
            let input_iter = tx.input_pts_iter();
            let tx_hash = tx.hash();

            self.old_inputs.extend(input_iter.cloned());
            self.old_outputs
                .insert(tx_hash.to_owned(), tx.outputs().len());
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ObsoleteCellSetOverlay<'a> {
    origin: &'a FnvHashMap<H256, TransactionMeta>,
    new: FnvHashMap<H256, TransactionMeta>,
    removed: FnvHashSet<H256>,
}

impl<'a> ObsoleteCellSetOverlay<'a> {
    pub fn get(&self, hash: &H256) -> Option<&TransactionMeta> {
        if self.removed.get(hash).is_some() {
            return None;
        }

        self.new.get(hash).or_else(|| self.origin.get(hash))
    }
}

type CellSetFilter = CountingBloomFilter<DefaultBuildHashKernels<RandomState>>;

pub struct CellSet {
    pub(crate) inner: FnvHashMap<H256, TransactionMeta>,
    filter: CellSetFilter,
    count: u64,
}

impl ::std::fmt::Debug for CellSet {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        write!(
            f,
            "CellSet {{ count: {}, /* fields omitted */ }}",
            self.count
        )
    }
}

impl CellSet {
    pub fn new() -> Self {
        // TODO optimize scalability
        let items_count: usize = 10_000_000;
        let bucket_size: u8 = 7;
        let fp_rate: f64 = 0.03;
        let build_hash_kernels = DefaultBuildHashKernels::new(rand::random(), RandomState::new());
        let filter =
            CountingBloomFilter::new(items_count, bucket_size, fp_rate, build_hash_kernels);
        CellSet {
            inner: FnvHashMap::default(),
            filter,
            count: 0,
        }
    }

    pub(crate) fn count(&self) -> u64 {
        self.count
    }

    pub(crate) fn insert_raw(&mut self, raw: &[u8]) {
        let (hash, index) = CellKey::deconstruct(raw);
        let key = (&hash, index);
        self.filter.insert(&key);
        self.count += 1;
    }

    pub(crate) fn insert_cells(&mut self, keys: &[(&H256, u32)]) {
        for key in keys {
            self.filter.insert(key);
        }
        self.count += keys.len() as u64;
    }

    pub(crate) fn delete_cells(&mut self, keys: &[(&H256, u32)]) {
        for key in keys {
            self.filter.remove(key);
        }
        self.count -= keys.len() as u64;
    }

    pub fn contains_probably(&self, tx_hash: &H256, index: u32) -> bool {
        let key = (tx_hash, index);
        self.filter.contains(&key)
    }

    pub fn obsolete_new_overlay<'a>(&'a self, diff: &CellSetDiff) -> ObsoleteCellSetOverlay<'a> {
        let mut new = FnvHashMap::default();
        let mut removed = FnvHashSet::default();

        for (hash, _) in &diff.old_outputs {
            if self.inner.get(&hash).is_some() {
                removed.insert(hash.clone());
            }
        }

        for (hash, (number, epoch, cellbase, len)) in diff.new_outputs.clone() {
            removed.remove(&hash);
            if cellbase {
                new.insert(hash, TransactionMeta::new_cellbase(number, epoch, len));
            } else {
                new.insert(hash, TransactionMeta::new(number, epoch, len));
            }
        }

        for old_input in &diff.old_inputs {
            if let Some(cell_input) = &old_input.cell {
                if let Some(meta) = self.inner.get(&cell_input.tx_hash) {
                    let meta = new
                        .entry(cell_input.tx_hash.clone())
                        .or_insert_with(|| meta.clone());
                    meta.unset_dead(cell_input.index as usize);
                }
            }
        }

        for new_input in &diff.new_inputs {
            if let Some(cell_input) = &new_input.cell {
                if let Some(meta) = self.inner.get(&cell_input.tx_hash) {
                    let meta = new
                        .entry(cell_input.tx_hash.clone())
                        .or_insert_with(|| meta.clone());
                    meta.set_dead(cell_input.index as usize);
                }
            }
        }

        ObsoleteCellSetOverlay {
            new,
            removed,
            origin: &self.inner,
        }
    }

    pub fn get(&self, h: &H256) -> Option<&TransactionMeta> {
        self.inner.get(h)
    }

    pub fn insert(
        &mut self,
        tx_hash: H256,
        number: u64,
        epoch: u64,
        cellbase: bool,
        outputs_len: usize,
    ) {
        if cellbase {
            self.inner.insert(
                tx_hash,
                TransactionMeta::new_cellbase(number, epoch, outputs_len),
            );
        } else {
            self.inner
                .insert(tx_hash, TransactionMeta::new(number, epoch, outputs_len));
        }
    }

    pub fn remove(&mut self, tx_hash: &H256) -> Option<TransactionMeta> {
        self.inner.remove(tx_hash)
    }

    pub fn mark_dead(&mut self, o: &OutPoint) {
        if let Some(cell) = &o.cell {
            if let Some(meta) = self.inner.get_mut(&cell.tx_hash) {
                meta.set_dead(cell.index as usize);
            }
        }
    }

    fn mark_live(&mut self, o: &OutPoint) {
        if let Some(cell) = &o.cell {
            if let Some(meta) = self.inner.get_mut(&cell.tx_hash) {
                meta.unset_dead(cell.index as usize);
            }
        }
    }

    pub fn update(&mut self, diff: CellSetDiff) {
        let CellSetDiff {
            old_inputs,
            old_outputs,
            new_inputs,
            new_outputs,
        } = diff;

        old_outputs.iter().for_each(|(h, _)| {
            self.remove(h);
        });

        old_inputs.iter().for_each(|o| {
            self.mark_live(o);
        });

        new_outputs
            .into_iter()
            .for_each(|(hash, (number, epoch, cellbase, len))| {
                self.insert(hash, number, epoch, cellbase, len);
            });

        new_inputs.iter().for_each(|o| {
            self.mark_dead(o);
        });
    }
}
