use ckb_app_config::{ExitCode, StatsArgs};
use ckb_async_runtime::Handle;
use ckb_launcher::SharedBuilder;
use ckb_shared::Shared;
use ckb_store::ChainStore;
use ckb_types::{
    core::{EpochNumber, RationalU256},
    utilities::compact_to_difficulty,
    U256,
};

pub fn list_epochs(args: StatsArgs, async_handle: Handle) -> Result<(), ExitCode> {
    let stats = Statics::build(args, async_handle)?;
    stats.display()?;
    Ok(())
}

struct Statics {
    shared: Shared,
    from: EpochNumber,
    to: EpochNumber,
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

        let from = args.from.unwrap_or(0);
        let to = args.to.unwrap_or(tip_number);

        if from > to {
            return Err(ExitCode::Cli);
        }

        Ok(Statics { shared, from, to })
    }

    /// Displays all epochs information.
    pub fn display(&self) -> Result<(), ExitCode> {
        let store = self.shared.store();

        println!("|---------+-----------+-----------+----------+--------+------------+------------------------------------------------+---------------------------------+--------------|");
        println!("|         |                   Block                   |  Compact   |                   Difficulty                   |     Difficulty  with Uncles     |   Hashrate   |");
        println!("|  Epoch  +-----------+-----------+----------+--------+            +--------------+------------------+--------------+---------------------------------+--------------|");
        println!("|         |   Start   |    End    |  Length  | Uncles |   Target   |    Block     |      Epoch       |   Changed    |      Epoch       |   Changed    |   Previous   |");
        println!("|---------+-----------+-----------+----------+--------+------------+--------------+------------------+--------------+---------------------------------+--------------|");

        let mut prev_epoch_difficulty = None;
        let mut prev_epoch_difficulty_with_uncles = None;
        let mut prev_epoch_last_block_total_uncles_count = {
            let ext = store
                .get_epoch_index(self.from)
                .and_then(|index| store.get_epoch_ext(&index))
                .ok_or(ExitCode::IO)?;
            let start_block_number = ext.start_number();
            let length = ext.length();
            let end_block_number = start_block_number + length - 1;
            store
                .get_block_hash(end_block_number)
                .and_then(|hash| store.get_block_ext(&hash))
                .ok_or(ExitCode::IO)?
                .total_uncles_count
        };

        for epoch_number in self.from..self.to {
            let ext = store
                .get_epoch_index(epoch_number)
                .and_then(|index| store.get_epoch_ext(&index))
                .ok_or(ExitCode::IO)?;

            let start_block_number = ext.start_number();
            let length = ext.length();
            let end_block_number = start_block_number + length - 1;
            let compact_target = ext.compact_target();
            let block_difficulty = compact_to_difficulty(compact_target);
            let epoch_last_block_total_uncles_count = store
                .get_block_hash(end_block_number)
                .and_then(|hash| store.get_block_ext(&hash))
                .ok_or(ExitCode::IO)?
                .total_uncles_count;
            let uncles_number =
                epoch_last_block_total_uncles_count - prev_epoch_last_block_total_uncles_count;

            let epoch_difficulty = block_difficulty.clone() * length;
            let epoch_difficulty_with_uncles =
                epoch_difficulty.clone() + block_difficulty.clone() * uncles_number;
            let previous_hashrate = ext.previous_epoch_hash_rate();

            let difficulty_changed_ratio = prev_epoch_difficulty
                .map(|prev| {
                    let curr = epoch_difficulty.clone();
                    const TMP: u32 = 1_000_000_000;
                    let ratio_u256 = (RationalU256::new(curr, prev) * U256::from(TMP)).into_u256();
                    f64::from(ratio_u256.0[0] as u32) / f64::from(TMP)
                })
                .unwrap_or(0.0);
            let difficulty_changed_ratio_with_uncles = prev_epoch_difficulty_with_uncles
                .map(|prev| {
                    let curr = epoch_difficulty.clone();
                    const TMP: u32 = 1_000_000_000;
                    let ratio_u256 = (RationalU256::new(curr, prev) * U256::from(TMP)).into_u256();
                    f64::from(ratio_u256.0[0] as u32) / f64::from(TMP)
                })
                .unwrap_or(0.0);

            let block_difficulty_str = format!("{}", block_difficulty);
            let epoch_difficulty_str = format!("{}", epoch_difficulty);
            let epoch_difficulty_with_uncles_str = format!("{}", epoch_difficulty_with_uncles);
            let previous_hashrate_str = format!("{}", previous_hashrate);

            println!("| {:7   } | {:9     } | {:9     } | {:8    } | {:6  } | {:10     } | {:>12      } | {:>16          } | {:12.9     } | {:>16          } | {:12.9     } | {:>12      } |",
                epoch_number,
                start_block_number,
                end_block_number,
                length,
                uncles_number,
                compact_target,
                block_difficulty_str,
                epoch_difficulty_str,
                difficulty_changed_ratio,
                epoch_difficulty_with_uncles_str,
                difficulty_changed_ratio_with_uncles,
                previous_hashrate_str,
            );

            prev_epoch_difficulty = Some(epoch_difficulty);
            prev_epoch_difficulty_with_uncles = Some(epoch_difficulty_with_uncles);
            prev_epoch_last_block_total_uncles_count = epoch_last_block_total_uncles_count;
        }

        Ok(())
    }
}
