mod basic;
mod block_assembler;
mod cell;
mod delay_verify;
mod dep_cell;
mod find_fork;
mod load_input_cell_data;
mod load_input_data_hash_cell;
mod non_contextual_block_txs_verify;
mod reward;
mod truncate;
mod uncle;
mod util;

// Unit Tests will be started in alphabetic order.
// We want this function to be the first.
#[test]
fn aaa_dummy_test() {
    use std::env;
    const MINIDUMP_UPLOAD_URL: &str = "MINIDUMP_UPLOAD_URL";
    if let Ok(url) = env::var(MINIDUMP_UPLOAD_URL) {
        let started = crashpad::start_crashpad(None, None, &url).unwrap_or(false);
        eprintln!("Crashpad: {}", started);
    }
}
