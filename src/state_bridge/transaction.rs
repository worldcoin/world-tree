use std::sync::Arc;
use std::time::Duration;

use ethers::providers::{Middleware, MiddlewareError, ProviderError};
use ethers::signers::{LocalWallet, WalletError};
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::{Bytes, Eip1559TransactionRequest, H160, H256};

use super::error::{StateBridgeError, TransactionError};

//Signs and sends transaction, bumps gas if necessary
pub async fn sign_and_send_transaction<M: Middleware>(
    mut tx: TypedTransaction,
    wallet_key: &LocalWallet,
    middleware: Arc<M>,
) -> Result<H256, TransactionError<M>> {
    let mut signed_tx = raw_signed_transaction(tx.clone(), wallet_key)?;
    loop {
        match middleware.send_raw_transaction(signed_tx.clone()).await {
            Ok(pending_tx) => {
                return Ok(pending_tx.tx_hash());
            }
            Err(err) => {
                let error_string = err.to_string();
                if error_string.contains("transaction underpriced") {
                    let eip1559_tx = tx.as_eip1559_mut().unwrap();
                    eip1559_tx.max_priority_fee_per_gas = Some(
                        eip1559_tx.max_priority_fee_per_gas.unwrap() * 150
                            / 100,
                    );
                    eip1559_tx.max_fee_per_gas =
                        Some(eip1559_tx.max_fee_per_gas.unwrap() * 150 / 100);

                    tx = eip1559_tx.to_owned().into();

                    signed_tx = raw_signed_transaction(tx.clone(), wallet_key)?;
                } else if error_string.contains("insufficient funds") {
                    return Err(TransactionError::InsufficientWalletFunds);
                } else {
                    return Err(err).map_err(TransactionError::MiddlewareError);
                }
            }
        }
    }
}

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

    let mut tx: TypedTransaction = Eip1559TransactionRequest::new()
        .data(calldata.clone())
        .to(to)
        .from(from)
        .chain_id(chain_id)
        .max_priority_fee_per_gas(max_priority_fee_per_gas)
        .max_fee_per_gas(max_fee_per_gas)
        .into();

    //match fill transaction, it will fail if the calldata fails
    middleware
        .fill_transaction(&mut tx, None)
        .await
        .map_err(TransactionError::MiddlewareError)?;

    tx.set_gas(tx.gas().unwrap() * 150 / 100);

    middleware
        .call(&tx, None)
        .await
        .map_err(TransactionError::MiddlewareError)?;

    Ok(tx)
}

pub async fn wait_for_transaction_receipt<M: Middleware>(
    tx_hash: H256,
    middleware: Arc<M>,
) -> Result<(), TransactionError<M>> {
    loop {
        if let Some(tx_receipt) = middleware
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(TransactionError::MiddlewareError)?
        {
            tracing::info!(?tx_receipt, "Tx receipt received");
            return Ok(());
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub fn raw_signed_transaction(
    tx: TypedTransaction,
    wallet_key: &LocalWallet,
) -> Result<Bytes, WalletError> {
    Ok(tx.rlp_signed(&wallet_key.sign_transaction_sync(&tx)?))
}
