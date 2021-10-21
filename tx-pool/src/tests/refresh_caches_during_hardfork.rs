use ckb_types::core::{capacity_bytes, Capacity, TransactionView};
use ckb_verification::{cache::Completed, MockedTxsVerifiedResult};

use crate::tests::utils;

#[test]
fn refresh_caches_during_hardfork() {
    let mut mock_chain = utils::MockChain::new();
    let tx_pool = mock_chain.tx_pool_controller().to_owned();

    let tx_pool_info = tx_pool.get_tx_pool_info().unwrap();
    assert_eq!(tx_pool_info.total_tx_size, 0);
    assert_eq!(tx_pool_info.total_tx_cycles, 0);

    let tx = TransactionView::new_advanced_builder().build();
    let tx_mocked_cycles_old = 100;
    let tx_mocked_cycles_new = 200;
    let tx_size = tx.data().serialized_size_in_block();

    MockedTxsVerifiedResult::mock_contextual(
        tx.hash(),
        Ok(Completed {
            cycles: tx_mocked_cycles_old,
            fee: capacity_bytes!(1),
        }),
    );

    tx_pool.submit_local_tx(tx.clone()).unwrap().unwrap();
    let tx_pool_entries = tx_pool.get_all_entry_info().unwrap();
    if let Some(tx_entry) = tx_pool_entries.pending.get(&tx.hash()) {
        assert_eq!(tx_entry.cycles, tx_mocked_cycles_old);
        let tx_pool_info = tx_pool.get_tx_pool_info().unwrap();
        assert_eq!(tx_pool_info.total_tx_size, tx_size);
        assert_eq!(tx_pool_info.total_tx_cycles, tx_mocked_cycles_old);
    }

    MockedTxsVerifiedResult::mock_contextual(
        tx.hash(),
        Ok(Completed {
            cycles: tx_mocked_cycles_new,
            fee: capacity_bytes!(1),
        }),
    );

    let block_stopped = utils::EPOCH_LENGTH * (utils::EPOCH_WHEN_HARDFORK + 1);
    for _ in 0..=block_stopped {
        mock_chain.tick();

        let epoch = mock_chain
            .current_snapshot()
            .tip_header()
            .epoch()
            .minimum_epoch_number_after_n_blocks(1);

        let cycles_expected = if epoch < utils::EPOCH_WHEN_HARDFORK {
            tx_mocked_cycles_old
        } else {
            tx_mocked_cycles_new
        };

        let tx_pool_entries = tx_pool.get_all_entry_info().unwrap();
        if let Some(tx_entry) = tx_pool_entries.pending.get(&tx.hash()) {
            assert_eq!(tx_entry.cycles, cycles_expected);
            let tx_pool_info = tx_pool.get_tx_pool_info().unwrap();
            assert_eq!(tx_pool_info.total_tx_size, tx_size);
            assert_eq!(tx_pool_info.total_tx_cycles, cycles_expected);
        }
    }

    let tx_pool_entries = tx_pool.get_all_entry_info().unwrap();
    if let Some(tx_entry) = tx_pool_entries.pending.get(&tx.hash()) {
        assert_eq!(tx_entry.cycles, tx_mocked_cycles_new);
        let tx_pool_info = tx_pool.get_tx_pool_info().unwrap();
        assert_eq!(tx_pool_info.total_tx_size, tx_size);
        assert_eq!(tx_pool_info.total_tx_cycles, tx_mocked_cycles_new);
    }
}
