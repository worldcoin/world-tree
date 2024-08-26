-- Metadata about on-chain transactions
-- Used to establish order of observed roots
-- And to fetch latest block number from which to sync
CREATE TABLE tx (
    id BIGSERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    tx_hash BYTEA NOT NULL
);

-- The canonical chain updates
CREATE TABLE canonical_updates (
    tx_id BIGINT PRIMARY KEY,
    pre_root BYTEA NOT NULL,
    post_root BYTEA NOT NULL,

    FOREIGN KEY (tx_id) REFERENCES tx (id)
);

-- The observed bridged updates
CREATE TABLE bridged_updates (
    tx_id BIGINT PRIMARY KEY,
    root BYTEA NOT NULL,

    FOREIGN KEY (tx_id) REFERENCES tx (id)
);

-- Flat leaf update storage
CREATE TABLE leaf_updates (
    id BIGSERIAL PRIMARY KEY,
    leaf_idx BIGINT NOT NULL,
    leaf BYTEA NOT NULL
);

-- Table to associate ranges of leaves with roots
CREATE TABLE leaf_batches (
    id BIGSERIAL PRIMARY KEY,
    root_id BIGINT NOT NULL,
    start_update_id BIGINT NOT NULL,
    num_updates BIGINT NOT NULL,

    FOREIGN KEY (root_id) REFERENCES canonical_updates (tx_id)
);
