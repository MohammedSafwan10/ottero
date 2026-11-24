-- v13: client-side Ottero Cloud sync state, wrapped collection keys, outbox,
-- and per-field HLC clocks (plan 06 §4).

CREATE TABLE sync_state (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    account_id        TEXT NOT NULL DEFAULT '',
    device_id         TEXT NOT NULL DEFAULT '',
    last_pull_seq     INTEGER NOT NULL DEFAULT 0 CHECK (last_pull_seq >= 0),
    last_hlc          TEXT,
    last_synced_at    INTEGER,
    needs_full_resync INTEGER NOT NULL DEFAULT 0 CHECK (needs_full_resync IN (0, 1))
);

INSERT INTO sync_state (id) VALUES (1);

CREATE TABLE sync_collection_keys (
    collection  TEXT PRIMARY KEY NOT NULL,
    wrapped_key BLOB NOT NULL,
    key_version INTEGER NOT NULL DEFAULT 1 CHECK (key_version >= 1),
    updated_at  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE sync_outbox (
    change_id      TEXT PRIMARY KEY NOT NULL,
    collection     TEXT NOT NULL,
    item_id        TEXT NOT NULL,
    hlc            TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK (schema_version >= 1),
    deleted        INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    fields_json    TEXT NOT NULL DEFAULT '{}',
    created_at     INTEGER NOT NULL
);

CREATE INDEX sync_outbox_hlc ON sync_outbox (hlc ASC);
CREATE INDEX sync_outbox_item ON sync_outbox (collection, item_id);

-- Per-field LWW clocks for structured records. Canonical HLC strings sort in
-- exact HLC order.
CREATE TABLE sync_field_clocks (
    collection TEXT NOT NULL,
    item_id    TEXT NOT NULL,
    field      TEXT NOT NULL,
    hlc        TEXT NOT NULL,
    PRIMARY KEY (collection, item_id, field)
) WITHOUT ROWID;
