-- The canonical chain updates
CREATE TABLE canonical_updates (
    tx_id BIGINT PRIMARY KEY,
    pre_root BYTEA NOT NULL,
    post_root BYTEA NOT NULL,

    FOREIGN KEY (tx_id) REFERENCES tx_meta (id)
);

-- The observed bridged updates
CREATE TABLE bridged_updates (
    tx_id BIGINT PRIMARY KEY,
    root BYTEA NOT NULL,

    FOREIGN KEY (tx_id) REFERENCES tx_meta (id)
);

-- Metadata about on-chain transactions
-- Used to establish order of observed roots
-- And to fetch latest block number from which to sync
CREATE TABLE tx_meta (
    id BIGSERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    tx_hash BYTEA NOT NULL
);

-- Flat leaf storage
CREATE TABLE leaves (
    id BIGSERIAL PRIMARY KEY,
    leaf BYTEA NOT NULL
);
