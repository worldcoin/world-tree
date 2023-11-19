use std::sync::Arc;

use ethers::providers::{JsonRpcClient, Middleware, PendingTransaction};
use ethers::signers::{LocalWallet, WalletError};
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::{
    Bytes, Eip1559TransactionRequest, TransactionReceipt, H160,
};
use tracing::instrument;

use super::error::TransactionError;

//Signs and sends transaction, bumps gas if necessary
#[instrument(skip(wallet_key, block_confirmations, middleware))]
pub async fn sign_and_send_transaction<M: Middleware>(
    tx: TypedTransaction,
    wallet_key: &LocalWallet,
    block_confirmations: usize,
    middleware: Arc<M>,
) -> Result<TransactionReceipt, TransactionError<M>> {
    tracing::info!("Signing tx");
    let signed_tx = raw_signed_transaction(tx.clone(), wallet_key)?;
    tracing::info!("Sending tx");
    match middleware.send_raw_transaction(signed_tx.clone()).await {
        Ok(pending_tx) => {
            let tx_hash = pending_tx.tx_hash();
            tracing::info!(?tx_hash, "Pending tx");

            return wait_for_tx_receipt(pending_tx, block_confirmations).await;
        }
        Err(err) => {
            let error_string = err.to_string();
            if error_string.contains("insufficient funds") {
                tracing::error!("Insufficient funds");
                return Err(TransactionError::InsufficientWalletFunds);
            } else {
                return Err(TransactionError::MiddlewareError(err));
            }
        }
    }
}

#[instrument(skip(middleware))]
pub async fn fill_and_simulate_eip1559_transaction<M: Middleware>(
    calldata: Bytes,
    to: H160,
    from: H160,
    chain_id: u64,
    middleware: Arc<M>,
) -> Result<TypedTransaction, TransactionError<M>> {
    let (max_fee_per_gas, max_priority_fee_per_gas) = middleware
        .estimate_eip1559_fees(None)
        .await
        .map_err(TransactionError::MiddlewareError)?;

    tracing::info!(
        ?max_fee_per_gas,
        ?max_priority_fee_per_gas,
        "Estimated gas fees"
    );

    let mut tx: TypedTransaction = Eip1559TransactionRequest::new()
        .data(calldata.clone())
        .to(to)
        .from(from)
        .chain_id(chain_id)
        .max_priority_fee_per_gas(max_priority_fee_per_gas)
        .max_fee_per_gas(max_fee_per_gas)
        .into();

    middleware
        .fill_transaction(&mut tx, None)
        .await
        .map_err(TransactionError::MiddlewareError)?;

    tx.set_gas(tx.gas().unwrap() * 150 / 100);

    let tx_gas = tx.gas().expect("Could not get tx gas");
    tracing::info!(?tx_gas, "Gas limit set");

    middleware
        .call(&tx, None)
        .await
        .map_err(TransactionError::MiddlewareError)?;

    tracing::info!("Successfully simulated tx");

    Ok(tx)
}

#[instrument]
pub async fn wait_for_tx_receipt<'a, M: Middleware, P: JsonRpcClient>(
    pending_tx: PendingTransaction<'a, P>,
    block_confirmations: usize,
) -> Result<TransactionReceipt, TransactionError<M>> {
    let pending_tx = pending_tx.confirmations(block_confirmations);
    let tx_hash = pending_tx.tx_hash();

    tracing::info!(
        ?tx_hash,
        ?block_confirmations,
        "Waiting for block confirmations"
    );

    if let Some(tx_receipt) =
        pending_tx.await.map_err(TransactionError::ProviderError)?
    {
        tracing::info!(?tx_receipt, "Tx receipt received");

        return Ok(tx_receipt);
    } else {
        return Err(TransactionError::TxReceiptNotFound(tx_hash));
    }
}

pub fn raw_signed_transaction(
    tx: TypedTransaction,
    wallet_key: &LocalWallet,
) -> Result<Bytes, WalletError> {
    Ok(tx.rlp_signed(&wallet_key.sign_transaction_sync(&tx)?))
}
