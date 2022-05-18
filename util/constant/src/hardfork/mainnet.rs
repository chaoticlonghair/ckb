/// The Chain Specification name.
pub const CHAIN_SPEC_NAME: &str = "ckb";

// TODO ckb2021 Update the epoch number for mainnet.
/// First epoch number for CKB v2021
pub const CKB2021_START_EPOCH: u64 = u64::MAX;

// TODO(light-client) update the block number.
/// First block which saves the MMR root hash into its header.
pub const MMR_ACTIVATED_BLOCK: u64 = u64::MAX;
