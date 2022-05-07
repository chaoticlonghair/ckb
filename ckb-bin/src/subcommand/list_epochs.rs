use ckb_app_config::{ExitCode, StatsArgs};
use ckb_async_runtime::Handle;
use ckb_launcher::SharedBuilder;
use ckb_merkle_mountain_range::leaf_index_to_mmr_size;
use ckb_shared::Shared;
use ckb_store::ChainStore;
use ckb_types::{core::BlockNumber, utilities::merkle_mountain_range::ChainRootMMR, U256};

pub fn list_epochs(args: StatsArgs, async_handle: Handle) -> Result<(), ExitCode> {
    let stats = Statics::build(args, async_handle)?;
    stats.display()?;
    Ok(())
}

struct Statics {
    shared: Shared,
    from: BlockNumber,
    to: BlockNumber,
}

impl Statics {
    pub fn build(args: StatsArgs, async_handle: Handle) -> Result<Self, ExitCode> {
        let shared_builder = SharedBuilder::new(
            &args.config.bin_name,
            args.config.root_dir.as_path(),
            &args.config.db,
            None,
            async_handle,
        )?;
        let (shared, _) = shared_builder.consensus(args.consensus).build()?;

        let tip_number = shared.snapshot().tip_number();

        let from = args.from.unwrap_or(1);
        let to = args.to.unwrap_or(tip_number);

        if from > to {
            return Err(ExitCode::Cli);
        }

        Ok(Statics { shared, from, to })
    }

    /// Displays all epochs information.
    pub fn display(&self) -> Result<(), ExitCode> {
        let snapshot = self.shared.snapshot();
        let store = self.shared.store();

        println!("Block Digests:");
        let mmr_size = leaf_index_to_mmr_size(self.to - 1);
        for num in 0..mmr_size {
            let digest = store.get_header_digest(num);
            if digest.is_none() {
                println!("- [{}]: Is  None", num);
            } else {
                println!("- [{}]: Not None", num);
            }
        }
        println!("End.");

        println!("Chain Root Hashes:");
        for num in self.from..=self.to {
            let mmr_size = leaf_index_to_mmr_size(num - 1);
            let mmr = ChainRootMMR::new(mmr_size, snapshot.as_ref());
            let chain_root = mmr.get_root().unwrap();
            let hash = chain_root.calc_mmr_hash();
            println!("- [{}] {:#x}", num, hash);
        }
        println!("End.");

        Ok(())
    }
}
