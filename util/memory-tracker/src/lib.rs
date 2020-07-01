use ckb_util::RwLock;
use std::sync::Arc;

lazy_static::lazy_static! {
    static ref INTERVAL: Arc<RwLock<u64>> = Arc::new(RwLock::new(0));
}

#[cfg(all(not(target_env = "msvc"), not(target_os = "macos")))]
mod process;
#[cfg(not(all(not(target_env = "msvc"), not(target_os = "macos"))))]
mod process {
    use std::{sync, thread, time};

    use ckb_logger::{error, info};
    use crossbeam_channel::{select, unbounded};

    use crate::{collections, rocksdb::TrackRocksDBMemory};

    pub fn track_current_process<Tracker: 'static + TrackRocksDBMemory + Sync + Send>(
        interval: u64,
        _: Option<sync::Arc<Tracker>>,
    ) {
        if interval == 0 {
            info!("track current process: disable");
        } else {
            info!(
                "track current process: enable (restricted; interval: {} seconds)",
                interval
            );
            crate::set_interval(interval);
            let (sender, receiver) = unbounded();
            collections::CONTROL_HANDLE.write().replace(sender);
            let wait_secs = time::Duration::from_secs(interval);
            if let Err(err) = thread::Builder::new()
                .name("MemoryTracker".to_string())
                .spawn(move || {
                    let mut now = time::Instant::now();
                    loop {
                        if now.elapsed().as_secs() >= interval {
                            now = time::Instant::now();
                            track_collections();
                        }
                        select! {
                            recv(receiver) -> (tag, record) => {
                                collections::STATISTICS.write().insert(tag, record);
                            }
                            default(time::Duration::from_secs(interval)) => {}
                        }
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
}
pub mod collections;
pub(crate) mod jemalloc;
pub mod rocksdb;
pub mod utils;

pub use jemalloc::jemalloc_profiling_dump;
pub use process::track_current_process;

pub fn interval() -> u64 {
    *INTERVAL.read()
}

pub(crate) fn set_interval(interval: u64) {
    *crate::INTERVAL.write() = interval;
}

pub fn track_current_process_simple(interval: u64) {
    track_current_process::<rocksdb::DummyRocksDB>(interval, None);
}
