#![allow(unused)]
// use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

// You can use calls not provided in RPC lib API using the generic `call` function.
// An example of using the `send` RPC call, which doesn't have exposed API.
// You can also use serde_json `Deserialize` derivation to capture the returned json result.
fn send(rpc: &Client, addr: &str) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!([{addr : 100 }]), // recipient address
        json!(null),            // conf target
        json!(null),            // estimate mode
        json!(null),            // fee rate in sats/vb
        json!(null),            // Empty option object
    ];

    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = rpc.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

static EMPTY_ADDRS: [bitcoincore_rpc::bitcoin::Address<
    bitcoincore_rpc::bitcoin::address::NetworkUnchecked,
>; 0] = [];

fn main() -> bitcoincore_rpc::Result<()> {
    // Connect to Bitcoin Core RPC
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Get blockchain info
    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {blockchain_info:?}");

    // Create/Load the wallets, named 'Miner' and 'Trader'. Have logic to optionally create/load them if they do not exist or not loaded already.
    // --- Wallet Creation/Loading ---
    for wallet_name in ["Miner", "Trader"] {
        let res = rpc.create_wallet(wallet_name, None, None, None, None);
        match res {
            Ok(_) => println!("Wallet '{wallet_name}' created."),
            Err(e) => {
                // If the error is "already exists", ignore it
                let msg = format!("{e}");
                if msg.contains("already exists") {
                    println!("Wallet '{wallet_name}' already exists.");
                } else {
                    return Err(e);
                }
            }
        }
    }
    // Instantiate Client objects for each wallet using wallet-specific URL
    let miner_wallet = Client::new(
        &format!("{}/wallet/{}", RPC_URL, "Miner"),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;
    let trader_wallet = Client::new(
        &format!("{}/wallet/{}", RPC_URL, "Trader"),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Generate spendable balances in the Miner wallet. How many blocks needs to be mined?
    // 1. Generate a mining address with label "Mining Reward"
    let mining_address = miner_wallet
        .get_new_address(Some("Mining Reward"), None)?
        .assume_checked();
    println!("Miner's mining address: {mining_address}");

    // 2. Mine blocks to this address until the wallet has a positive balance
    // Coinbase rewards require 100 confirmations to mature before they are spendable.
    // So, we need to mine at least 101 blocks to have a positive spendable balance.
    let mut balance = miner_wallet.get_balance(None, None)?.to_btc();
    let mut blocks_mined = 0;
    while balance <= 0.0 {
        miner_wallet.generate_to_address(1, &mining_address)?;
        blocks_mined += 1;
        balance = miner_wallet.get_balance(None, None)?.to_btc();
    }
    println!("Blocks mined until positive balance: {blocks_mined}");
    // ---
    // Coinbase rewards (mining rewards) require 100 confirmations before they can be spent. This is a consensus rule to prevent chain reorganizations from invalidating recent coinbase spends. That's why the wallet balance is not immediately available after mining a block; you must mine 100 more blocks before the reward is mature and spendable.
    // ---
    println!("Miner wallet balance: {balance} BTC");

    // Load Trader wallet and generate a new address
    // 1. Generate a receiving address for Trader with label "Received"
    let trader_address = trader_wallet
        .get_new_address(Some("Received"), None)?
        .assume_checked();
    println!("Trader's receiving address: {trader_address}");

    // 2. Send 20 BTC from Miner to Trader
    let txid = miner_wallet.send_to_address(
        &trader_address,
        Amount::from_btc(20.0)?,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;
    println!("Sent 20 BTC from Miner to Trader. Transaction ID: {txid}");

    // Check transaction in mempool
    // 1. Fetch the unconfirmed transaction from the mempool and print the result
    let mempool_entry = miner_wallet.get_mempool_entry(&txid)?;
    println!("Mempool entry for txid {txid}: {mempool_entry:#?}");

    // 2. Mine 1 block to confirm the transaction
    miner_wallet.generate_to_address(1, &mining_address)?;
    println!("Mined 1 block to confirm the transaction.");

    // Extract all required transaction details
    use bitcoincore_rpc::bitcoin::Txid;
    use std::path::Path;

    // 1. Get the confirmed transaction details
    let tx_info = miner_wallet.get_transaction(&txid, None)?;
    let block_hash = tx_info
        .info
        .blockhash
        .expect("Transaction should be confirmed in a block");
    let block = miner_wallet.get_block_info(&block_hash)?;
    let block_height = block.height;

    // 2. Get the raw transaction and decode it
    let raw_tx = miner_wallet.get_raw_transaction(&txid, Some(&block_hash))?;
    let decoded_tx = miner_wallet.decode_raw_transaction(&raw_tx, None)?;

    // 3. Find input address and amount (from previous output)
    let input = &decoded_tx.vin[0];
    let prev_txid = input.txid.expect("Input should have txid");
    let prev_vout = input.vout.expect("Input should have vout") as usize;
    let prev_tx = miner_wallet.get_raw_transaction(&prev_txid, None)?;
    let prev_decoded = miner_wallet.decode_raw_transaction(&prev_tx, None)?;
    let prev_output = &prev_decoded.vout[prev_vout];
    let input_addresses = &prev_output.script_pub_key.addresses;
    let miner_input_address: String = input_addresses
        .first()
        .map(|a| format!("{}", a.clone().assume_checked()))
        .unwrap_or_default();
    let miner_input_amount: f64 = prev_output.value.to_btc();

    // 4. Find outputs: trader's output, miner's change
    let mut trader_output_address: String = String::new();
    let mut trader_output_amount: f64 = 0.0;
    let mut miner_change_address: String = String::new();
    let mut miner_change_amount: f64 = 0.0;
    println!("Decoded transaction outputs:");
    for vout in &decoded_tx.vout {
        if let Some(addr) = &vout.script_pub_key.address {
            let addr_str = addr.clone().assume_checked().to_string();
            println!("  Address: {addr_str}, Value: {:.8}", vout.value.to_btc());
            if addr_str == trader_address.to_string() {
                trader_output_address = addr_str.clone();
                trader_output_amount = vout.value.to_btc();
            } else {
                // Check if this address belongs to the miner wallet
                let info = miner_wallet.get_address_info(&addr.clone().assume_checked());
                if let Ok(address_info) = info {
                    if address_info.is_mine.unwrap_or(false) {
                        miner_change_address = addr_str.clone();
                        miner_change_amount = vout.value.to_btc();
                    }
                }
            }
        }
    }

    println!("trader_output_address: {trader_output_address}");
    println!("trader_output_amount: {trader_output_amount:.8}");
    println!("miner_change_address: {miner_change_address}");
    println!("miner_change_amount: {miner_change_amount:.8}");

    // 5. Calculate transaction fee: input - (output1 + output2)
    let tx_fee = miner_input_amount - (trader_output_amount + miner_change_amount);

    // 6. Write to ../out.txt in the required format
    let out_path = Path::new("../out.txt");
    let mut out_file = File::create(out_path)?;
    writeln!(out_file, "{txid}")?;
    writeln!(out_file, "{miner_input_address}")?;
    writeln!(out_file, "{miner_input_amount:.8}")?;
    writeln!(out_file, "{trader_output_address}")?;
    writeln!(out_file, "{trader_output_amount:.8}")?;
    writeln!(out_file, "{miner_change_address}")?;
    writeln!(out_file, "{miner_change_amount:.8}")?;
    writeln!(out_file, "{:.8}", tx_fee.abs())?;
    writeln!(out_file, "{block_height}")?;
    writeln!(out_file, "{block_hash}")?;
    println!("Transaction details written to ../out.txt");

    Ok(())
}
