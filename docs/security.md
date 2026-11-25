# Security

## What we protect

Notes, tasks, events, habits, and passwords/2FA secrets on the user's own devices,
and later on a self-hosted sync server the user controls.

## Threats in scope

- Someone with a copy of the data folder or a backup (lost laptop, cloud backup).
- A compromised or curious sync server (must only ever see ciphertext).
- Other local apps reading app files (partially; see limits).
- Accidental leaks: logs, crash reports, error messages, clipboard.

Out of scope: malware running as the user while the vault is unlocked, a
compromised OS, physical attacks on unlocked RAM.

## Notes, tasks, calendar, habits

- Notes and tasks are stored **unencrypted** in SQLite in the user data folder,
  like Obsidian. Users should rely on FileVault/BitLocker for at-rest protection.
- Holiday data and personal content stay local. Updates contact GitHub; opening
  external links uses the system browser. No telemetry. No `unsafe` code.
- Errors and logs never include note bodies or passwords.
- User search input is escaped before FTS5 (`codec::fts_query`); all SQL uses
  bound parameters.

## Update boundary

The shared catalog is minisign-verified with a pinned public key before parse.
Windows packages stream through a 32 KiB buffer with signed length/SHA-256
validation before Velopack installs them. macOS Sparkle separately verifies its
inline-signed XML appcast and Ed25519 archive before extraction. The appcast is
pinned to the same release as the common catalog. Persisted highest verified
version rejects older catalog replay; this is not signed metadata expiration.

Automatic checks/downloads default on. Checks start after the first window and
retry with bounded backoff. Restart refuses failed note/Developer/Passwords saves,
unapplied secret imports, open task sheets and active focus timers. A private
SQLite online-backup snapshot includes WAL data before the handoff. Notes/tasks
in that snapshot are unencrypted, like the original database; vault rows remain
encrypted. There is no automatic database rollback or new-version health monitor.

Sparkle owns Mac installation/relaunch; Velopack owns Windows installation/relaunch.
The old custom extraction/swap installer is removed. A per-data-directory OS lock
prevents two application processes; locks release on crash without stale-timeout
heuristics. A stale legacy cache is not trusted or executed.

Update keys are separate from publisher identities. The Sparkle private seed is
stored outside the repo in the owner's protected release-key directory and as a
GitHub Actions secret; only its public key ships. CI verifies both private/public
key correspondences before generating the complete candidate. No existing signing
identity or system Keychain is used. Mac bundles use ad-hoc sealing, not Developer
ID/notarization; Windows has no Authenticode certificate. Initial OS warnings
remain. Compromised release credentials can authorize malicious updates.

See [plan 13](plans/13-production-updates.md) and
[ADR 0008](decisions/0008-updater-engines.md) for verified tests and remaining
publication/recovery gates. Interrupted-install and early-crash recovery must not
be described as proven merely because a clean upgrade passed.

## Vault design

- Master password → **Argon2id** v1.3, default m=64 MiB, t=3, p=4 (RFC 9106
  option 2), random 16-byte salt. Params stored per vault and bounded on read
  (19 MiB–1 GiB, t 2–64, p 1–16) so a tampered header can't weaken or hang unlock.
- Optional 128-bit **secret key** (like 1Password) mixed in, so a leaked
  server database is useless even with a weak password. Planned.
- Derived key unwraps a random **vault key**; items are sealed with
  **XChaCha20-Poly1305**, fresh 24-byte random nonce each time, record ID as
  associated data. Format is versioned (`"KS" | v1 | nonce | ct | tag`).
- Keys are `SecretKey` (zeroize on drop, redacted Debug, not Clone).
- Auto-lock on idle/sleep, clipboard auto-clear for copied passwords, no
  secrets in search index or previews.

## Vault as built

- Hierarchy and format: ADR 0005. Storage holds the header and sealed blobs
  only; titles, URLs and favorites are encrypted too.
- Unlock/setup/password change derive on a background thread; the UI never
  blocks on Argon2. A result arriving after the user locked again is ignored.
- **Locking** (manual ⌃⌘L, idle timer, background, waking from sleep) drops
  the vault key and every decrypted item, wipes unsaved drafts and the
  search field, and closes an open generator.
- **Idle** is measured on the wall clock (`SystemTime`), because `Instant`
  stops while macOS sleeps. A 10 s tick that arrives ≥ 60 s late means the
  Mac slept: the vault locks on wake.
- **Not detected:** the macOS lock screen without sleep. GPUI exposes no
  hook and `unsafe` FFI is forbidden, so use "Lock when Ottero is in the
  background" for that.
- **Randomness** never falls back: if the OS RNG fails, nothing is generated
  (the generator shows an error; a stuck source stops after 1,000 tries).
- **Clipboard:** copied secrets are cleared after 15/30/90 s, only if the
  clipboard still holds them. macOS clipboard history apps may still keep a copy.
- **Memory:** decrypted items zero on drop. Text in input fields lives in
  GPUI's input state and its undo history, which Ottero clears but can't
  zero; that's within "malware while unlocked" (out of scope).
- **Health** (weak/reused/old) is computed locally with zxcvbn; no breach
  lookups, nothing leaves the device.
- **Encrypted backup** (Settings → Passwords, File menu, ⌘K) writes the
  header and sealed items to an `.otvault` file (`.cfvault` under Calmfold, `.ksvault` under Keepstone; all still open) (the archive format, ADR 0006)
  atomically and reads it back. Nothing is decrypted to write it; it opens
  with the master password at the time of export (not the recovery key).
  Restore needs an unlocked vault, adds copies with new IDs, and skips items
  identical to live ones.

## Developer projects and environment secrets

- Developer project names, links, local folder references and setup notes are
  ordinary **unencrypted** local app data, like tasks and notes. HTTP(S) links
  reject embedded usernames/passwords. Do not put credentials in those fields.
- Environment sets reuse Passwords' vault key, encrypted items, auto-lock,
  clipboard clearing, recovery and encrypted backups. Variable names, values,
  project association and environment labels are inside the encrypted item;
  neither SQLite metadata nor FTS receives them.
- Environment items use item-data format v2; regular items stay v1. The vault
  encryption envelope/KDF are unchanged. Older clients must be upgraded to read
  v2 items. Edits and backup recovery are tested with more than 30 variables.
- Import is bounded to 1 MiB, 512 variables and 64 KiB per value; duplicate
  names and malformed input are rejected with safe line-number errors.
  Parsing never executes commands or expands references. Canonical export
  preserves literal values in Ottero's documented dialect; other dotenv
  runtimes can differ. Comments and original file layout are not preserved.
- The masked single-line editor represents line breaks as `\n`, returns as
  `\r`, tabs as `\t` and backslashes as `\\`. Pasting a value escapes these
  automatically. Copy/export decode the literal value; multiline imports are
  tested through UI editing and saving.
- File-picker results carry a draft generation. Closing/replacing a draft or
  locking invalidates late results. Lock clears rows, previews and reveals;
  the existing GPUI input-memory limitations above still apply.
- Plaintext export needs confirmation and a chosen location. A temporary file
  is written/synced and published without overwriting an existing file; Unix
  permissions are 0600. This exported file is no longer protected by the vault.
  Prefer encrypted `.otvault` backups for storage/transfer.
- Export takes an unlocked snapshot only after confirmation/path selection;
  disk writing and sync run in the background. Locking before a picker returns
  prevents the export. Once an approved write starts, it completes that snapshot;
  locking cannot recall plaintext the user has chosen to export.
- Revealing a value displays literal characters (including multiline content).
  The escaped representation is for masked editing only. Failed validation or
  an unapplied import keeps the draft open when switching Passwords items.

## Keychain policy

- Unsigned `cargo run` binaries change their code signature on every build,
  so macOS asks for the login keychain password each time (the "wants to
  access key … Safe Storage" prompt). To avoid that:
  - **Development builds never use the system keychain.** Quick-unlock
    secrets in debug builds go to a dev-only store or are simply not saved.
  - **Release builds** (later) use the macOS Data Protection keychain with a
    proper Developer ID signature, and Windows Credential Manager / DPAPI.
- Do **not** code-sign with whatever identity happens to be on the machine.
  Signing setup will be a deliberate release step with the owner's own identity.

## Ottero Cloud account (as built, plan 06 step 3)

- **Keys.** One password plus a 128-bit Secret Key (ADR 0010, format and
  vectors in ADR 0011, `ottero_crypto::account`). Argon2id (256 MiB, t=3)
  runs on the device in the background; HKDF splits the result into an auth
  key (sent as the server's "password", which the server hashes again) and a
  wrap key (never sent). The server stores the salt, the hashed auth key and
  the account key wrapped twice (by the wrap key and by the recovery code). It
  never sees the password, the Secret Key, the recovery code or the account key.
  A `kdf` string other than `argon2id:m=262144,t=3,p=1:v1` is refused.
- **Sign-in on a new device** derives the auth key from the password and the
  typed Secret Key, signs in, downloads the wrapped key and opens it. A wrong
  password or Secret Key fails at the server or at the AEAD tag; the message
  never says which.
- **On this device** `account.json` in the data folder (mode 0600, written
  atomically) holds the email, session token, salt, the wrapped account key,
  the cached plan and the Secret Key per email. The account key itself is only
  in memory: after a restart Settings → Account asks for the password to
  "Unlock Ottero Cloud" (offline: it opens the cached wrapped key). Sign out
  drops the token and the in-memory key and keeps the Secret Key, so signing in
  again needs only the password. A damaged file is moved aside, never
  overwritten.
- **Where the secrets live** (ADR 0012). Windows release builds keep the
  session token and Secret Keys in Windows Credential Manager (Local
  persistence, encrypted with the Windows login); `account.json` then holds
  only public values. macOS release builds keep them in `account.json` until
  releases are Developer ID signed, because keychain items are bound to the
  code signature and Ottero's ad-hoc signature changes with every update.
  Development builds always use the file. The Secret Key alone can't sign in
  or decrypt anything without the password; the token alone opens no data.
- **Emergency Kit** is a printable HTML page written with mode 0600 to a path
  the user picks, then opened in the browser to print or save as PDF. The
  recovery code appears only during sign-up; later kits hold email and
  Secret Key and require the password first.
- **Network.** `OTTERO_ACCOUNT_URL` (debug default `http://127.0.0.1:8787`,
  release default `https://account.nexdark.com`; release builds refuse
  anything but https). ureq with rustls and timeouts. Tokens, codes, keys and
  response bodies are never logged; errors are fixed sentences.
- **Two-step verification** (TOTP, Better Auth `twoFactor`: SHA-1, 30 s,
  6 digits, 10 single-use backup codes, lockout for 15 minutes after 5 wrong
  codes; no email or SMS second factor). It guards *signing in*: a new device,
  or this one after sign-out or a revoked session. Unlocking after a restart
  still needs only the password. Turning it on, off or getting new backup
  codes asks for the password, which is checked on the device first (it must
  open the wrapped account key), then sent only as the derived auth key.
  - The QR code (`otpauth://` link) and setup key are shown once in the
    dialog and kept only in that dialog's memory; backup codes too, until
    Done. None of them are written to disk. Copy puts them on the clipboard
    and clears it after the vault's clipboard delay.
  - Sign-in with two-step on returns a signed `better-auth.two_factor`
    cookie (no session). The app keeps it in memory for the code step and
    sends it with `Origin` set to the account server's own origin (Better
    Auth's CSRF check needs a trusted origin on cookie requests). It is
    never stored or logged and is dropped when the step is left.
  - Turning it on or off replaces the session server-side; the replacement
    token is saved at once and starts a new epoch, so late answers for the
    old token are dropped. If such an answer arrives after the user signed
    out or switched account, the session it created is signed out again.
- Passwords moves under the account key when the user is signed in (next
  section, ADR 0014).

## Passwords under the Ottero account (ADR 0014)

- **Not signed in:** unchanged. The vault has its own master password,
  recovery key and auto-lock.
- **Wrapping:** the vault key is sealed again with
  `HKDF-SHA256(account_key, info="ottero vault v1")`, in the vault envelope
  with AAD `"ottero vault key account v1"` (`ottero_crypto::vault_link`,
  fixed vectors checked with independent implementations). No item is
  re-encrypted.
- **Linking** (signed in, account unlocked, existing vault) asks for the
  master password once, also when the vault is already unlocked, so nobody at
  an unlocked computer can tie the vault to their own account. One SQLite
  transaction stores the account wrapping and removes the master-password
  wrapping (compare-and-swap on the wrapping just opened); a vault without a
  recovery key gets one in the same write and it is shown once. A failure or
  crash before the commit leaves the master password working. The master
  password is **retired, not kept**: a dormant second opener would stay
  open to offline guessing after the user was told it was gone.
- **Schema guarantee** (migration 0012): a vault always has the master
  password, or the account wrapping together with a recovery key.
- **New vault while signed in:** no master password; account wrapping plus
  a recovery key from the start.
- **One unlock:** opening the account key (sign-in, the account password
  after a restart, any password check) opens a linked vault too. Locking
  Passwords (manual, idle, sleep, background) also drops the account key;
  locking the account locks Passwords.
- **Sign out / session ended:** Passwords locks at once and the account key
  is dropped. The link stays; signing in to the same account reopens it.
- **Another account on this device:** the vault records the (normalized)
  email it is linked to. Signed in as another account, it is not tried,
  nothing changes, and the locked screen names the right account and offers
  the recovery key. Same email but a key that doesn't open it (a re-created
  account, a changed file): AEAD failure, explained, nothing changed.
- **Recovery key** still opens a linked vault. With an unlocked account
  signed in, the vault then opens with that account; otherwise the user sets
  a new master password and the account wrapping is removed in the same
  write. Signing out or switching accounts never brings back a master
  password by itself.
- **Backups and archives** of a linked vault are archive version 2: no
  master password; they hold the account and recovery wrappings and open with
  that account or the recovery key. Master-password vaults still write
  version 1 unchanged. Older app versions refuse both a schema-12 database
  and version-2 files rather than misreading them.
- **On disk:** `vault_meta` holds the email and the account wrapping
  (ciphertext). The account key, the derived link key and the vault key are
  never written. Development builds still never use the system keychain.
- **Limits:** a linked vault is as strong as the account password + Secret
  Key, or the recovery key. Where the Secret Key is in `account.json` (macOS,
  development), a stolen data folder leaves the account password (Argon2id
  256 MiB) as the effective secret. Losing the account password, the
  Emergency Kit and the vault recovery key together loses the vault.

## Sync (Phase 4)

End-to-end encrypted: the server stores opaque blobs and metadata needed for
ordering only. Transport is HTTPS/WSS (TLS) only. Account auth is separate from
the vault key.
