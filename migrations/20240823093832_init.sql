-- Metadata about on-chain transactions
-- Used to establish order of observed roots
-- And to fetch latest block number from which to sync
CREATE TABLE tx (
    -- This id exists for the purpose of ordering these txs
    -- and linking different tables together
    -- the ids are strictly increasing, but they don't
    -- maintain strict ordering across chains/contracts
    id BIGSERIAL PRIMARY KEY,
    chain_id BIGINT NOT NULL,
    block_number BIGINT NOT NULL,
    tx_hash BYTEA NOT NULL
);

-- The canonical chain updates
CREATE TABLE updates (
    id BIGSERIAL PRIMARY KEY,
    tx_id BIGINT NOT NULL,

    pre_root BYTEA NOT NULL UNIQUE,
    post_root BYTEA NOT NULL UNIQUE,

    FOREIGN KEY (tx_id) REFERENCES tx (id)
);

-- Table to monitor roots on each chain
CREATE TABLE roots (
    tx_id BIGINT PRIMARY KEY,
    root BYTEA NOT NULL,

    FOREIGN KEY (tx_id) REFERENCES tx (id),
    FOREIGN KEY (root) REFERENCES updates (post_root)
);

-- Flat leaf update storage
CREATE TABLE leaf_updates (
    id BIGINT PRIMARY KEY,
    leaf_idx BIGINT NOT NULL,
    leaf BYTEA NOT NULL
);

-- Table to associate ranges of leaves with roots
CREATE TABLE leaf_batches (
    update_id BIGINT PRIMARY KEY,
    start_id BIGINT NOT NULL,
    end_id BIGINT NOT NULL,

    FOREIGN KEY (update_id) REFERENCES updates (id),
    FOREIGN KEY (start_id) REFERENCES leaf_updates (id),
    FOREIGN KEY (end_id) REFERENCES leaf_updates (id)
);
