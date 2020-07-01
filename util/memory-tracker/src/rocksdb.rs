use std::fmt;

use ckb_db::internal::ops::{GetColumnFamilys, GetProperty, GetPropertyCF};
use ckb_logger::trace;

use crate::utils::{sum_int_values, PropertyValue};

// Ref: https://github.com/facebook/rocksdb/wiki/Memory-usage-in-RocksDB
pub struct RocksDBMemoryStatistics {
    pub(crate) total_memory: PropertyValue<u64>,
    pub(crate) block_cache_usage: PropertyValue<u64>,
    pub(crate) estimate_table_readers_mem: PropertyValue<u64>,
    pub(crate) cur_size_all_mem_tables: PropertyValue<u64>,
    pub(crate) block_cache_pinned_usage: PropertyValue<u64>,
    pub(crate) block_cache_capacity: PropertyValue<u64>,
}

impl fmt::Display for RocksDBMemoryStatistics {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("RocksDB")
            .field("total", &self.total_memory)
            .field("cache", &self.block_cache_usage)
            .field("readers", &self.estimate_table_readers_mem)
            .field("memtables", &self.cur_size_all_mem_tables)
            .field("pinned", &self.block_cache_pinned_usage)
            .field("cache-capacity", &self.block_cache_capacity)
            .finish()
    }
}

impl Default for RocksDBMemoryStatistics {
    fn default() -> Self {
        Self {
            total_memory: PropertyValue::Null,
            block_cache_usage: PropertyValue::Null,
            estimate_table_readers_mem: PropertyValue::Null,
            cur_size_all_mem_tables: PropertyValue::Null,
            block_cache_pinned_usage: PropertyValue::Null,
            block_cache_capacity: PropertyValue::Null,
        }
    }
}

pub trait TrackRocksDBMemory {
    fn gather_memory_stats(&self) -> RocksDBMemoryStatistics {
        let block_cache_usage = self.gather_int_values("rocksdb.block-cache-usage");
        let estimate_table_readers_mem =
            self.gather_int_values("rocksdb.estimate-table-readers-mem");
        let cur_size_all_mem_tables = self.gather_int_values("rocksdb.cur-size-all-mem-tables");
        let block_cache_pinned_usage = self.gather_int_values("rocksdb.block-cache-pinned-usage");
        let total_memory = sum_int_values(&[
            block_cache_usage.clone(),
            estimate_table_readers_mem.clone(),
            cur_size_all_mem_tables.clone(),
            block_cache_pinned_usage.clone(),
        ]);
        let block_cache_capacity = self.gather_int_values("rocksdb.block-cache-capacity");
        RocksDBMemoryStatistics {
            total_memory,
            block_cache_usage,
            estimate_table_readers_mem,
            cur_size_all_mem_tables,
            block_cache_pinned_usage,
            block_cache_capacity,
        }
    }
    fn gather_int_values(&self, key: &str) -> PropertyValue<u64>;
}

pub struct DummyRocksDB;

impl TrackRocksDBMemory for DummyRocksDB {
    fn gather_int_values(&self, _: &str) -> PropertyValue<u64> {
        PropertyValue::Null
    }
}

impl<RocksDB> TrackRocksDBMemory for RocksDB
where
    RocksDB: GetColumnFamilys + GetProperty + GetPropertyCF,
{
    fn gather_int_values(&self, key: &str) -> PropertyValue<u64> {
        let mut values = Vec::new();
        let value_default = self
            .property_int_value(key)
            .map_err(|err| format!("{}", err))
            .into();
        trace!("{}(_): {}", key, value_default);
        values.push(value_default);
        for (cf_name, cf) in self.get_cfs() {
            let value_col = self
                .property_int_value_cf(cf, key)
                .map_err(|err| format!("{}", err))
                .into();
            trace!("{}({}): {}", key, cf_name, value_col);
            if cf_name == "default" {
                continue;
            }
            values.push(value_col);
        }
        sum_int_values(&values)
    }
}
