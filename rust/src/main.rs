// Enable unused code for development flexibility
#![allow(unused)]
use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;

// Configuration for connecting to the Bitcoin Core node
const NODE_URL: &str = "http://127.0.0.1:18443"; // Regtest RPC endpoint
const NODE_USER: &str = "alice";
const NODE_PASS: &str = "password";

// Custom RPC call for 'send' method, not directly exposed in the library
fn send_transaction(rpc: &Client, address: &str) -> bitcoincore_rpc::Result<String> {
    let params = [
        json!([{address : 100 }]), // Target address for sending
        json!(null),               // Confirmation target (default)
        json!(null),               // Fee estimation mode
        json!(null),               // Fee rate in satoshis per virtual byte
        json!(null),               // Additional options (none)
    ];

    #[derive(Deserialize)]
    struct TransactionResult {
        complete: bool,
        txid: String,
    }
    let result = rpc.call::<TransactionResult>("send", &params)?;
    assert!(result.complete, "Transaction failed to complete");
    Ok(result.txid)
}

// Empty address array for type safety
static NO_ADDRESSES: [bitcoincore_rpc::bitcoin::Address<
    bitcoincore_rpc::bitcoin::address::NetworkUnchecked,
>; 0] = [];

fn main() -> bitcoincore_rpc::Result<()> {
    // Establish connection to Bitcoin Core node
    let client = Client::new(
        NODE_URL,
        Auth::UserPass(NODE_USER.to_string(), NODE_PASS.to_string()),
    )?;

    // Retrieve and display blockchain information
    let chain_info = client.get_blockchain_info()?;
    println!("Chain Info: {chain_info:#?}");

    // Initialize or load wallets 'Miner' and 'Trader'
    for wallet in ["Miner", "Trader"] {
        match client.create_wallet(wallet, None, None, None, None) {
            Ok(_) => println!("Created wallet: {wallet}"),
            Err(e) if e.to_string().contains("already exists") => {
                println!("Wallet {wallet} already loaded")
            }
            Err(e) => return Err(e),
        }
    }

    // Connect to wallet-specific RPC endpoints
    let miner_client = Client::new(
        &format!("{}/wallet/{}", NODE_URL, "Miner"),
        Auth::UserPass(NODE_USER.to_string(), NODE_PASS.to_string()),
    )?;
    let trader_client = Client::new(
        &format!("{}/wallet/{}", NODE_URL, "Trader"),
        Auth::UserPass(NODE_USER.to_string(), NODE_PASS.to_string()),
    )?;

    // Generate funds in Miner wallet by mining blocks
    // Obtain a new address for mining rewards
    let miner_addr = miner_client
        .get_new_address(Some("Mining Reward"), None)?
        .assume_checked();
    println!("Miner address for rewards: {miner_addr}");

    // Mine blocks until Miner has spendable funds
    // Note: Coinbase outputs need 100 confirmations to mature
    let mut balance = miner_client.get_balance(None, None)?.to_btc();
    let mut blocks = 0;
    while balance <= 0.0 {
        miner_client.generate_to_address(1, &miner_addr)?;
        blocks += 1;
        balance = miner_client.get_balance(None, None)?.to_btc();
    }
    println!("Mined {blocks} blocks to achieve balance: {balance} BTC");

    // Generate a receiving address for Trader wallet
    let trader_addr = trader_client
        .get_new_address(Some("Payment"), None)?
        .assume_checked();
    println!("Trader payment address: {trader_addr}");

    // Transfer 20 BTC from Miner to Trader
    let tx_id = miner_client.send_to_address(
        &trader_addr,
        Amount::from_btc(20.0)?,
        None,
        None,
        None,
        None,
        None,
        None,
    )?;
    println!("Transferred 20 BTC to Trader. TxID: {tx_id}");

    // Verify transaction in mempool
    let mempool_data = miner_client.get_mempool_entry(&tx_id)?;
    println!("Mempool data for TxID {tx_id}: {mempool_data:#?}");

    // Confirm transaction by mining one block
    miner_client.generate_to_address(1, &miner_addr)?;
    println!("Mined a block to confirm transaction");

    // Extract transaction details for analysis
    use bitcoincore_rpc::bitcoin::Txid;
    use std::path::Path;

    // Fetch confirmed transaction details
    let tx_details = miner_client.get_transaction(&tx_id, None)?;
    let block_hash = tx_details
        .info
        .blockhash
        .expect("Expected transaction to be in a block");
    let block_info = miner_client.get_block_info(&block_hash)?;
    let block_height = block_info.height;

    // Decode raw transaction
    let raw_tx = miner_client.get_raw_transaction(&tx_id, Some(&block_hash))?;
    let decoded_tx = miner_client.decode_raw_transaction(&raw_tx, None)?;

    // Extract input details
    let input = &decoded_tx.vin[0];
    let prev_txid = input.txid.expect("Input must have a txid");
    let prev_vout = input.vout.expect("Input must have a vout") as usize;
    let prev_tx = miner_client.get_raw_transaction(&prev_txid, None)?;
    let prev_decoded = miner_client.decode_raw_transaction(&prev_tx, None)?;
    let prev_output = &prev_decoded.vout[prev_vout];
    let input_addr = prev_output
        .script_pub_key
        .addresses
        .first()
        .map(|a| a.clone().assume_checked().to_string())
        .unwrap_or_default();
    let input_amount = prev_output.value.to_btc();

    // Extract output details: Trader's output and Miner's change
    let mut trader_out_addr = String::new();
    let mut trader_out_amount = 0.0;
    let mut miner_change_addr = String::new();
    let mut miner_change_amount = 0.0;
    println!("Transaction outputs:");
    for output in &decoded_tx.vout {
        if let Some(addr) = &output.script_pub_key.address {
            let addr_str = addr.clone().assume_checked().to_string();
            let value = output.value.to_btc();
            println!("  Address: {addr_str}, Amount: {value:.8} BTC");
            if addr_str == trader_addr.to_string() {
                trader_out_addr = addr_str.clone();
                trader_out_amount = value;
            } else if miner_client
                .get_address_info(&addr.clone().assume_checked())
                .map(|info| info.is_mine.unwrap_or(false))
                .unwrap_or(false)
            {
                miner_change_addr = addr_str.clone();
                miner_change_amount = value;
            }
        }
    }

    println!("Trader output address: {trader_out_addr}");
    println!("Trader output amount: {trader_out_amount:.8}");
    println!("Miner change address: {miner_change_addr}");
    println!("Miner change amount: {miner_change_amount:.8}");

    // Calculate the transaction fee
    let fee = input_amount - (trader_out_amount + miner_change_amount);

    // Write transaction details to output file
    let output_path = Path::new("../out.txt");
    let mut file = File::create(output_path)?;
    writeln!(file, "{}", tx_id)?;
    writeln!(file, "{}", input_addr)?;
    writeln!(file, "{:.8}", input_amount)?;
    writeln!(file, "{}", trader_out_addr)?;
    writeln!(file, "{:.8}", trader_out_amount)?;
    writeln!(file, "{}", miner_change_addr)?;
    writeln!(file, "{:.8}", miner_change_amount)?;
    writeln!(file, "{:.8}", fee.abs())?;
    writeln!(file, "{}", block_height)?;
    writeln!(file, "{}", block_hash)?;
    println!("Saved transaction details to ../out.txt");

    Ok(())
}
