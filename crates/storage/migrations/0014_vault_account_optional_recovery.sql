-- v14: Ottero Cloud accounts already have an Account Emergency Kit, so
-- Cloud-linked vaults no longer require a separate local vault recovery key.
-- Local-only (master-password) vaults continue to support recovery keys as before.
--
-- Invariants:
-- * master-password columns are all set or all NULL;
-- * account_email and account_wrapped are both set or both NULL;
-- * recovery_salt and recovery_wrapped are both set or both NULL;
-- * a vault always keeps at least one opener: master password OR account key.
CREATE TABLE vault_meta_v14 (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    version           INTEGER NOT NULL,
    salt              BLOB,
    kdf_memory        INTEGER,
    kdf_time          INTEGER,
    kdf_lanes         INTEGER,
    wrapped_key       BLOB,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    recovery_salt     BLOB,
    recovery_wrapped  BLOB,
    account_email     TEXT,
    account_wrapped   BLOB,
    CHECK ((salt IS NULL) = (wrapped_key IS NULL)
       AND (kdf_memory IS NULL) = (wrapped_key IS NULL)
       AND (kdf_time IS NULL) = (wrapped_key IS NULL)
       AND (kdf_lanes IS NULL) = (wrapped_key IS NULL)),
    CHECK ((account_email IS NULL) = (account_wrapped IS NULL)),
    CHECK ((recovery_salt IS NULL) = (recovery_wrapped IS NULL)),
    CHECK (wrapped_key IS NOT NULL OR account_wrapped IS NOT NULL)
);

INSERT INTO vault_meta_v14 (id, version, salt, kdf_memory, kdf_time, kdf_lanes, wrapped_key,
                            created_at, updated_at, recovery_salt, recovery_wrapped,
                            account_email, account_wrapped)
SELECT id, version, salt, kdf_memory, kdf_time, kdf_lanes, wrapped_key,
       created_at, updated_at, recovery_salt, recovery_wrapped,
       account_email, account_wrapped
FROM vault_meta;

DROP TABLE vault_meta;
ALTER TABLE vault_meta_v14 RENAME TO vault_meta;
