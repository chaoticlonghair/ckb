//! Mocked scripts.
//!
//! Each mocked script execution can be represented as 16 bytes:
//! - First 8 bytes are the result of the script execution.
//!   - First 4 bytes are the script error type.
//!   - Second 4 bytes are the details of the script error.
//! - Second 8 bytes are the consumed cycles when returns a result.
//!
//! For any mocked script:
//! - The first 16 bytes are used as script execution result with VM0.
//! - The second 16 bytes are used as script execution result with VM1.

#![doc(hidden)]

use std::{
    collections::HashSet,
    convert::{TryFrom, TryInto},
    sync::Arc,
};

use ckb_traits::{CellDataProvider, HeaderProvider};
use ckb_types::{
    core::{Cycle, ScriptHashType},
    packed::{self, Byte32},
};
use ckb_util::RwLock;
use ckb_vm::{snapshot::Snapshot, SupportMachine};

use crate::{
    error::ScriptError,
    types::{Machine, ResumableMachine, ScriptGroup, ScriptVersion},
    verify::{ChunkState, TransactionScriptsVerifier},
};

lazy_static::lazy_static! {
    static ref MOCKED_SCRIPT: Arc<RwLock<MockedScripts>> = MockedScripts::init();
}

/// Stores all mocked scripts hashes.
pub struct MockedScripts {
    data_hashes: HashSet<Byte32>,
    type_hashes: HashSet<Byte32>,
}

enum MockedResult {
    Completed(Cycle),
    Suspended(Cycle),
    Error(ScriptError),
}

impl MockedScripts {
    fn init() -> Arc<RwLock<Self>> {
        let ret = Self {
            data_hashes: HashSet::default(),
            type_hashes: HashSet::default(),
        };
        Arc::new(RwLock::new(ret))
    }

    /// Inserts a data hash for a mocked script.
    pub fn insert_data_hash(hash: Byte32) -> bool {
        MOCKED_SCRIPT.write().data_hashes.insert(hash)
    }

    /// Inserts a type hash for a mocked script.
    pub fn insert_type_hash(hash: Byte32) -> bool {
        MOCKED_SCRIPT.write().type_hashes.insert(hash)
    }

    /// Removes a data hash for a mocked script.
    pub fn remove_data_hash(hash: &Byte32) -> bool {
        MOCKED_SCRIPT.write().data_hashes.remove(hash)
    }

    /// Removes a type hash for a mocked script.
    pub fn remove_type_hash(hash: &Byte32) -> bool {
        MOCKED_SCRIPT.write().type_hashes.remove(hash)
    }

    /// Checks if the input hash is data hash of a mocked script.
    pub fn contains_data_hash(hash: &Byte32) -> bool {
        MOCKED_SCRIPT.read().data_hashes.contains(hash)
    }

    /// Checks if the input hash is type hash of a mocked script.
    pub fn contains_type_hash(hash: &Byte32) -> bool {
        MOCKED_SCRIPT.read().type_hashes.contains(hash)
    }

    /// Removes all mocked scripts hashes.
    pub fn clear() {
        let mut guard = MOCKED_SCRIPT.write();
        guard.data_hashes.clear();
        guard.type_hashes.clear();
    }
}

impl MockedScripts {
    /// Checks if a script is a mocked script.
    pub(crate) fn contains(script: &packed::Script) -> bool {
        ScriptHashType::try_from(script.hash_type())
            .map(|script_hash_type| {
                if matches!(script_hash_type, ScriptHashType::Type) {
                    Self::contains_type_hash(&script.code_hash())
                } else {
                    Self::contains_data_hash(&script.code_hash())
                }
            })
            .unwrap_or(false)
    }

    /// Runs a verifier.
    pub(crate) fn run<'a, DL>(
        verifier: &TransactionScriptsVerifier<'a, DL>,
        script_group: &'a ScriptGroup,
        max_cycles: Cycle,
    ) -> Result<Cycle, ScriptError>
    where
        DL: CellDataProvider + HeaderProvider,
    {
        let script_version = verifier.select_version(&script_group.script)?;
        let mocked_result = Self::execute(script_version, &script_group.script, max_cycles);
        match mocked_result {
            MockedResult::Completed(cycles) => Ok(cycles),
            MockedResult::Suspended(cycles) => Err(ScriptError::ExceededMaximumCycles(cycles)),
            MockedResult::Error(error) => Err(error),
        }
    }

    /// Runs a verifier in chunk mode.
    pub(crate) fn chunk_run<'a, DL>(
        verifier: &TransactionScriptsVerifier<'a, DL>,
        mut machine: Machine<'a>,
        script_group: &'a ScriptGroup,
        max_cycles: Cycle,
        snap: &Option<(Snapshot, Cycle)>,
    ) -> Result<ChunkState<'a>, ScriptError>
    where
        DL: CellDataProvider + HeaderProvider,
    {
        let script_version = verifier.select_version(&script_group.script)?;
        let mocked_result = if let Some((_sp, current_cycle)) = snap {
            Self::resume(
                script_version,
                &script_group.script,
                max_cycles,
                *current_cycle,
            )
        } else {
            Self::execute(script_version, &script_group.script, max_cycles)
        };

        match mocked_result {
            MockedResult::Completed(cycles) => Ok(ChunkState::Completed(cycles)),
            MockedResult::Suspended(cycles) => {
                machine.machine.set_cycles(cycles);
                Ok(ChunkState::suspended(ResumableMachine::new(machine, true)))
            }
            MockedResult::Error(error) => Err(error),
        }
    }

    /// Executes a mocked script.
    fn execute(version: ScriptVersion, script: &packed::Script, max_cycles: Cycle) -> MockedResult {
        Self::resume(version, script, max_cycles, 0)
    }

    /// Resumes a mocked script.
    fn resume(
        version: ScriptVersion,
        script: &packed::Script,
        max_cycles: Cycle,
        current_cycles: Cycle,
    ) -> MockedResult {
        let args = script.args().raw_data();
        let index = match version {
            ScriptVersion::V0 => 0,
            ScriptVersion::V1 => 16,
        };
        if args.len() < index + 16 {
            let errmsg = format!(
                "no enough args (length is {} but requires {}..{})",
                args.len(),
                index,
                index + 16
            );
            return MockedResult::Error(ScriptError::Mock(errmsg));
        }
        let result_slice = &args[index..index + 16];
        let result_type = u64::from_le_bytes(result_slice[0..8].try_into().unwrap());
        let result_cycles = u64::from_le_bytes(result_slice[8..].try_into().unwrap());
        if result_cycles <= max_cycles {
            if result_type == 0 {
                MockedResult::Completed(current_cycles + result_cycles)
            } else {
                MockedResult::Error(ScriptError::from_mocked(result_slice))
            }
        } else {
            MockedResult::Suspended(current_cycles + max_cycles)
        }
    }
}

impl ScriptError {
    /// Converts a slice (8 bytes) into a script error.
    fn from_mocked(err_slice: &[u8]) -> Self {
        let err_type = u32::from_le_bytes(err_slice[0..4].try_into().unwrap());
        let err_details = u32::from_le_bytes(err_slice[4..8].try_into().unwrap());
        let errmsg = format!("unknown error [{:#010x}, {:#010x}]", err_type, err_details);
        ScriptError::Mock(errmsg)
    }
}
