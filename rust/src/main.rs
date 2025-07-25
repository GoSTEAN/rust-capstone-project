#![allow(unused)]
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;
use std::str::FromStr;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

// Helper function to create or load a wallet
fn create_or_load_wallet(rpc: &Client, wallet_name: &str) -> bitcoincore_rpc::Result<()> {
    // First try to load the wallet
    let load_result = rpc.load_wallet(wallet_name);

    match load_result {
        Ok(_) => {
            println!("Wallet '{wallet_name}' loaded successfully");
            Ok(())
        }
        Err(_) => {
            // If loading failed, try to create the wallet
            println!("Wallet '{wallet_name}' not found, creating new wallet");
            let create_result = rpc.create_wallet(wallet_name, None, None, None, None);
            match create_result {
                Ok(_) => {
                    println!("Wallet '{wallet_name}' created successfully");
                    Ok(())
                }
                Err(e) => {
                    println!("Failed to create wallet '{wallet_name}': {e}");
                    Err(e)
                }
            }
        }
    }
}

// Helper function to switch to a specific wallet
fn get_wallet_client(wallet_name: &str) -> bitcoincore_rpc::Result<Client> {
    let wallet_url = format!("{RPC_URL}/wallet/{wallet_name}");
    Client::new(
        &wallet_url,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )
}

// Helper function to send Bitcoin from one wallet to another
fn send_bitcoin(
    from_wallet: &Client,
    to_address: &str,
    amount: f64,
) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!({to_address: amount}), // recipient address and amount
        json!(null),                 // conf target
        json!(null),                 // estimate mode
        json!(null),                 // fee rate in sats/vb
        json!(null),                 // Empty option object
    ];

    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = from_wallet.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

fn main() -> bitcoincore_rpc::Result<()> {
    // Connect to Bitcoin Core RPC
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Get blockchain info
    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {blockchain_info:?}");

    // Create/Load the wallets, named 'Miner' and 'Trader'
    let miner_wallet = "Miner";
    let trader_wallet = "Trader";

    create_or_load_wallet(&rpc, miner_wallet)?;
    create_or_load_wallet(&rpc, trader_wallet)?;

    // Get wallet clients
    let miner_client = get_wallet_client(miner_wallet)?;
    let trader_client = get_wallet_client(trader_wallet)?;

    // Generate spendable balances in the Miner wallet
    println!("Mining blocks to generate spendable balance...");
    let mining_address = miner_client.get_new_address(Some("Mining Reward"), None)?;
    let mining_address_checked = mining_address.assume_checked();
    println!("Generated mining address with label 'Mining Reward': {mining_address_checked}");
    let blocks = rpc.generate_to_address(101, &mining_address_checked)?;
    println!("Mined {} blocks", blocks.len());

    // Check miner balance
    let miner_balance = miner_client.get_balance(None, None)?;
    println!("Miner wallet balance: {} BTC", miner_balance.to_btc());

    // Create a receiving address labeled "Received" from Trader wallet
    let trader_address = trader_client.get_new_address(Some("Received"), None)?;
    let trader_address_checked = trader_address.assume_checked();
    println!("Generated trader address with label 'Received': {trader_address_checked}");

    // Send 20 BTC from Miner to Trader
    println!("Sending 20 BTC from Miner to Trader...");
    let txid_str = send_bitcoin(&miner_client, &trader_address_checked.to_string(), 20.0)?;
    println!("Transaction sent with TXID: {txid_str}");

    // Parse TXID using bitcoincore_rpc's bitcoin crate
    let txid = bitcoincore_rpc::bitcoin::Txid::from_str(&txid_str).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to parse TXID: {e}"),
        )
    })?;

    // Get mempool entry
    println!("Fetching unconfirmed transaction from mempool...");
    let mempool_entry = rpc.call::<serde_json::Value>("getmempoolentry", &[json!(txid_str)])?;
    println!("Mempool entry: {mempool_entry:?}");

    // Also check general mempool info
    let mempool_info = rpc.get_mempool_info()?;
    println!("Mempool info: {mempool_info:?}");

    // Mine 1 block to confirm the transaction
    println!("Mining 1 block to confirm transaction...");
    let new_blocks = rpc.generate_to_address(1, &mining_address_checked)?;
    println!("Mined block: {:?}", new_blocks[0]);

    // Extract all required transaction details
    let transaction = miner_client.get_transaction(&txid, None)?;
    println!("Transaction details: {transaction:?}");

    // Get block information
    let block_hash = transaction
        .info
        .blockhash
        .expect("Transaction should be in a block");
    let block = rpc.get_block(&block_hash)?;
    let block_height = rpc.get_block_header_info(&block_hash)?.height;

    // Get raw transaction for more details
    let raw_tx = rpc.get_raw_transaction(&txid, None)?;
    let decoded_tx = rpc.decode_raw_transaction(&raw_tx, None)?;

    // Get current balances
    let final_miner_balance = miner_client.get_balance(None, None)?;
    let final_trader_balance = trader_client.get_balance(None, None)?;
    println!(
        "Final Miner Wallet Balance: {} BTC",
        final_miner_balance.to_btc()
    );
    println!(
        "Final Trader Wallet Balance: {} BTC",
        final_trader_balance.to_btc()
    );

    // Extract input and output details for the specific format required
    let mut miner_input_address = String::new();
    let mut miner_input_amount = 0.0;

    // Get input details - we need to look up the previous transaction to get the input address
    if let Some(input) = decoded_tx.vin.first() {
        // Use the txid and vout fields directly instead of previous_output
        if let (Some(prev_txid), Some(vout)) = (&input.txid, &input.vout) {
            if let Ok(prev_tx) = rpc.get_raw_transaction(prev_txid, None) {
                if let Ok(prev_decoded) = rpc.decode_raw_transaction(&prev_tx, None) {
                    if let Some(prev_output) = prev_decoded.vout.get(*vout as usize) {
                        if let Some(address) = &prev_output.script_pub_key.address {
                            miner_input_address = format!("{}", address.clone().assume_checked());
                            miner_input_amount = prev_output.value.to_btc();
                        }
                    }
                }
            }
        }
    }

    // Extract output details
    let mut trader_output_address = String::new();
    let mut trader_output_amount = 0.0;
    let mut miner_change_address = String::new();
    let mut miner_change_amount = 0.0;

    // Identify which output is the trader's and which is the miner's change
    for output in &decoded_tx.vout {
        if let Some(address) = &output.script_pub_key.address {
            if format!("{}", address.clone().assume_checked()) == trader_address_checked.to_string()
            {
                trader_output_address = format!("{}", address.clone().assume_checked());
                trader_output_amount = output.value.to_btc();
            } else {
                // This is the change output back to miner
                miner_change_address = format!("{}", address.clone().assume_checked());
                miner_change_amount = output.value.to_btc();
            }
        }
    }

    // Calculate fee
    let fee = transaction.fee.map_or(0.0, |f| f.to_btc().abs());

    // Write the data to out.txt in the exact format required by the test
    let output_data = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        txid,
        miner_input_address,
        miner_input_amount,
        trader_output_address,
        trader_output_amount,
        miner_change_address,
        miner_change_amount,
        -fee, // Negative fee as shown in sample
        block_height,
        block_hash
    );

    // Write to file
    let mut file = File::create("out.txt")?;
    file.write_all(output_data.as_bytes())?;
    println!("Transaction details written to out.txt");

    // Debug: Print the content we're writing to verify format
    println!("\n=== OUTPUT FILE CONTENT ===");
    println!("{output_data}");
    println!("============================");

    println!("\n=== SUMMARY ===");
    println!("✓ Created/loaded Miner and Trader wallets");
    println!("✓ Generated mining address with label 'Mining Reward'");
    println!("✓ Mined 101 blocks to generate spendable balance (coinbase maturity requirement)");
    println!("✓ Generated trader address with label 'Received'");
    println!("✓ Sent 20 BTC from Miner to Trader");
    println!("✓ Fetched unconfirmed transaction from mempool using getmempoolentry");
    println!("✓ Confirmed transaction with 1 additional block");
    println!("✓ Exported transaction details to out.txt in required format");

    Ok(())
}
