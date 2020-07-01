use std::fmt;

use crate::utils::PropertyValue;

#[cfg(all(
    not(target_env = "msvc"),
    not(target_os = "macos"),
    feature = "profiling"
))]
mod inner {
    use std::{ffi, mem, ptr};

    use ckb_logger::info;

    pub fn jemalloc_profiling_dump(filename: &str) -> Result<(), String> {
        let mut filename0 = format!("{}\0", filename);
        let opt_name = "prof.dump";
        let opt_c_name = ffi::CString::new(opt_name).unwrap();
        info!("jemalloc profiling dump: {}", filename);
        unsafe {
            jemalloc_sys::mallctl(
                opt_c_name.as_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut filename0 as *mut _ as *mut _,
                mem::size_of::<*mut ffi::c_void>(),
            );
        }

        Ok(())
    }
}
#[cfg(not(all(
    not(target_env = "msvc"),
    not(target_os = "macos"),
    feature = "profiling"
)))]
mod inner {
    pub fn jemalloc_profiling_dump(_: &str) -> Result<(), String> {
        Err("jemalloc profiling dump: unsupported".to_string())
    }
}

pub struct JeMallocMemoryStatistics {
    pub(crate) allocated: PropertyValue<u64>,
    pub(crate) resident: PropertyValue<u64>,
    pub(crate) active: PropertyValue<u64>,
    pub(crate) mapped: PropertyValue<u64>,
    pub(crate) retained: PropertyValue<u64>,
    pub(crate) metadata: PropertyValue<u64>,
}

impl fmt::Display for JeMallocMemoryStatistics {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("JeMalloc")
            .field("allocated", &self.allocated)
            .field("resident", &self.resident)
            .field("active", &self.active)
            .field("mapped", &self.mapped)
            .field("retained", &self.retained)
            .field("metadata", &self.metadata)
            .finish()
    }
}

pub use inner::jemalloc_profiling_dump;
