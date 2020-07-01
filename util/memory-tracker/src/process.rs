use std::{sync, thread, time};

use ckb_logger::{debug, error, info, trace};
use crossbeam_channel::{select, unbounded};
use futures::executor::block_on;
use heim::units::information::byte;
use jemalloc_ctl::{epoch, stats};

use crate::{
    collections,
    jemalloc::JeMallocMemoryStatistics,
    rocksdb::TrackRocksDBMemory,
    utils::{HumanReadableSize, PropertyValue},
};

macro_rules! je_mib {
    ($key:ty) => {
        if let Ok(value) = <$key>::mib() {
            value
        } else {
            error!("failed to lookup jemalloc mib for {}", stringify!($key));
            return;
        }
    };
}

macro_rules! mib_read {
    ($mib:ident) => {
        match $mib.read() {
            Ok(value) => PropertyValue::Value(value as u64),
            Err(err) => {
                let error = format!(
                    "failed to read jemalloc stats for {}: {}",
                    stringify!($mib),
                    err
                );
                PropertyValue::Error(error)
            }
        }
    };
}

pub fn track_current_process<Tracker: 'static + TrackRocksDBMemory + Sync + Send>(
    interval: u64,
    tracker_opt: Option<sync::Arc<Tracker>>,
) {
    if interval == 0 {
        info!("track current process: disable");
    } else {
        info!(
            "track current process: enable (interval: {} seconds)",
            interval
        );
        crate::set_interval(interval);
        let (sender, receiver) = unbounded();
        collections::CONTROL_HANDLE.write().replace(sender);

        let je_epoch = je_mib!(epoch);
        // Bytes allocated by the application.
        let allocated = je_mib!(stats::allocated);
        // Bytes in physically resident data pages mapped by the allocator.
        let resident = je_mib!(stats::resident);
        // Bytes in active pages allocated by the application.
        let active = je_mib!(stats::active);
        // Bytes in active extents mapped by the allocator.
        let mapped = je_mib!(stats::mapped);
        // Bytes in virtual memory mappings that were retained
        // rather than being returned to the operating system
        let retained = je_mib!(stats::retained);
        // Bytes dedicated to jemalloc metadata.
        let metadata = je_mib!(stats::metadata);

        if let Err(err) = thread::Builder::new()
            .name("MemoryTracker".to_string())
            .spawn(move || {
                trace!("MemoryTracker is running ...");
                if let Ok(process) = block_on(heim::process::current()) {
                    let pid = process.pid();
                    let mut now = time::Instant::now();
                    loop {
                        if now.elapsed().as_secs() >= interval {
                            now = time::Instant::now();
                            if je_epoch.advance().is_err() {
                                error!("failed to refresh the jemalloc stats");
                                return;
                            }
                            if let Ok(memory) = block_on(process.memory()) {
                                // Resident set size, amount of non-swapped physical memory.
                                let rss: HumanReadableSize = memory.rss().get::<byte>().into();
                                // Virtual memory size, total amount of memory.
                                let virt: HumanReadableSize = memory.vms().get::<byte>().into();

                                let jemalloc_stats = JeMallocMemoryStatistics {
                                    allocated: mib_read!(allocated),
                                    resident: mib_read!(resident),
                                    active: mib_read!(active),
                                    mapped: mib_read!(mapped),
                                    retained: mib_read!(retained),
                                    metadata: mib_read!(metadata),
                                };

                                if let Some(tracker) = tracker_opt.clone() {
                                    let rocksdb_stats = tracker.gather_memory_stats();
                                    debug!(
                                        "CurrentProcess {{ \
                                                pid: {}, rss: {}, virt: {}, \
                                                allocator: {}, database: {}
                                            }}",
                                        pid, rss, virt, jemalloc_stats, rocksdb_stats,
                                    );
                                } else {
                                    debug!(
                                        "CurrentProcess {{ \
                                                pid: {}, rss: {}, virt: {}, \
                                                allocator: {}
                                            }}",
                                        pid, rss, virt, jemalloc_stats
                                    );
                                }
                            } else {
                                error!(
                                    "failed to fetch the memory information about current process"
                                );
                            }
                            collections::track_collections();
                        }
                        select! {
                            recv(receiver) -> item => {
                                if let Ok((tag, record)) = item {
                                    collections::STATISTICS.write().insert(tag, record);
                                }
                            }
                            default(time::Duration::from_secs(interval)) => {
                            }
                        }
                    }
                } else {
                    error!("failed to track the currently running program");
                }
            })
        {
            error!(
                "failed to spawn the thread to track current process: {}",
                err
            );
        }
    }
}
