# 06 Accounts, Ottero Cloud and sync

Status: design, awaiting owner review. Decisions: ADR 0010.

Ottero stays fully usable without an account. **Ottero Cloud** is the paid,
optional layer: an account, end-to-end encrypted sync across devices in real
time, encrypted cloud backup and note history. The server never sees note,
task, event, habit, vault or project content, and never sees the password.

## 1. Pieces

| Piece | Tech | Where | Repo |
| --- | --- | --- | --- |
| Desktop app | Rust/GPUI (this repo): native sign-up/sign-in, sync engine | user's computer | `ottero` (public) |
| Account & sync services | Hosted account API, Rust sync server, billing & email | `account.nexdark.com` / `sync.nexdark.com` | private |

The protocol crate shared by the app and the sync server lives in this repo
(`crates/sync-protocol`, no IO) so the open-source client documents exactly
what it sends.

## 2. Sign-up and sign-in (inside the app)

Sign-in is a native form in the app, not a web page, because the password is
also the root of the encryption key and must never leave the device in a form
that opens data. The browser is used only for hosted subscription checkout and
billing management.

### Key derivation (one password + Secret Key, 1Password model)

```
salt        = random 16 bytes, created at sign-up, stored by the server, public
secret_key  = random 128-bit, created on the device at sign-up, never sent;
              kept in the OS keychain (release builds) and the Emergency Kit
master_key  = Argon2id(password, salt, m=256 MiB, t=3, p=1)  XOR  HKDF(secret_key)
auth_key    = HKDF-SHA256(master_key, info="ottero auth v1")   # sent to log in
wrap_key    = HKDF-SHA256(master_key, info="ottero wrap v1")   # never leaves device
server stores: Argon2id(auth_key, server_salt, m=64 MiB) — never auth_key itself
```

- One password for everything: the account password also unlocks the vault
  section. There is no second vault password for Cloud users.
- The server can verify a login but cannot derive `wrap_key`. Because the
  Secret Key never reaches the server, a stolen database can't be attacked
  offline even for a weak password.
- A device needs the Secret Key once: from a signed-in device by QR approval
  (§3), or typed from the **Emergency Kit** (PDF: email, Secret Key, recovery
  code). After that the device keeps it and never asks again.
- **Unlock:** the app locks after inactivity or restart, as today. Unlock with
  the account password, or Touch ID / Windows Hello on release builds. Notes,
  tasks and the calendar stay open; the lock covers the vault section.
- **Local-only users** (no account) keep today's vault master password.
  Turning Cloud on asks for it once, re-wraps the vault key under
  `account_key`, and from then on the account password unlocks the vault.
- Argon2id parameters are stored per account (`kdf` record) so they can be
  raised later; old accounts upgrade on next sign-in.
- Passwords: 10+ characters, checked against the top breached-password list
  on the device (no network lookup); strength meter in the form.

### Flow

1. **Sign up:** email + password → device derives keys → API creates the
   account (unverified) → server sends a 6-digit verification code (10 min, 5 tries) →
   user types it in the app → account verified → app shows the **Emergency
   Kit** (Secret Key + recovery code, printable PDF) and asks the user to
   confirm they saved it.
2. **Sign in:** email → API returns the account's public `salt`/`kdf` (a
   fixed fake salt for unknown emails, so emails can't be enumerated) → device
   derives `auth_key` (needs the Secret Key: QR approval or Emergency Kit)
   → API checks it → session → the device unwraps `account_key` locally.
3. **Two-factor (optional):** TOTP authenticator app or passkey as a second
   step at sign-in. Strongly suggested in the app after the first sync.
4. **Sessions:** short-lived access token (JWT, 15 min, EdDSA, verified by the
   sync server against the API's JWKS) + rotating refresh token (60 days,
   reuse detection signs out the whole family). One session per device, listed
   and revocable. Release builds keep the refresh token in the OS keychain
   (macOS Data Protection keychain, Windows Credential Manager); development
   builds keep it in an encrypted file in the dev data folder and never touch
   the keychain (security.md, Keychain policy).
5. **Forgot password:** email code proves the inbox → the user enters the
   **recovery key** → the device unwraps the account key and re-wraps it with
   the new password. Without the recovery key the user can only **reset
   Cloud**: the cloud copy is deleted and re-uploaded from a device that still
   has the data. The app says this plainly at sign-up.

### Abuse protection

Per-IP and per-account rate limits on sign-in, sign-up and code checks;
exponential back-off after failures; sign-in alerts by email for a new device;
disposable-email block list on sign-up; all auth endpoints behind TLS only
(HSTS), CSRF-protected cookies for the website, bearer tokens for the app.

## 3. Encryption key hierarchy

```
account_key (random 256-bit, created at sign-up on the device)
 ├─ wrapped by wrap_key           → stored on server (sign in on a new device)
 ├─ wrapped by recovery_key       → stored on server (forgot password)
 └─ wrapped to each device's X25519 public key (HPKE, RFC 9180)
collection keys (random, one per data type: notes, tasks, events, habits,
  focus, projects, settings, vault), wrapped by account_key
item payload = XChaCha20-Poly1305(collection_key, plaintext,
                 AAD = account_id | collection | item_id | schema_version)
```

- The existing local vault (Argon2id + XChaCha20-Poly1305, ADR 0003) keeps
  its data key. Turning Cloud on wraps that key by `account_key` (no item is
  re-encrypted); the vault's own master password is then retired on that
  device and the account password unlocks it. Built 2026-11-18 (ADR 0014):
  `HKDF(account_key, "ottero vault v1")` wraps the vault key locally; the
  master password is retired, the vault recovery key is required and kept. Vault items sync as encrypted
  blobs and stay locked until the app is unlocked on each device.
- Recovery code: 32 random bytes plus a checksum, shown as 8 groups of 7
  Crockford characters (ADR 0011 explains why not BIP39 words), derived into
  `recovery_wrap_key` with HKDF; the server stores only its wrap.
- Changing the password re-wraps `account_key` only; no data is re-encrypted.
  Removing a compromised device rotates `account_key` and the collection keys
  (re-wrap, then background re-encryption).
- New crypto lives in `crates/crypto` with test vectors (crypto/AGENTS.md):
  HKDF, X25519 + HPKE (`hpke` crate, audited suites only), BIP39 encoding.

### New device by QR approval

1. New device enters the email and password and shows a QR code: its
   device ID, X25519 public key and a 6-digit check code.
2. A signed-in device scans or types the code, shows "Approve *MacBook Air*
   in Mumbai?", and on approval sends the Secret Key and `account_key` sealed
   to the new device's public key (HPKE).
3. The new device opens it; the server only relayed ciphertext. Without
   another device, the user types the Secret Key from the Emergency Kit.

## 4. Sync protocol

- **Change log:** every local write appends `{collection, item_id, hlc,
  fields_changed}` to a new `sync_outbox` table (migration `000N_sync.sql`).
  HLC = hybrid logical clock (wall time + counter + device id), so clock skew
  can't reorder edits.
- **Records** (tasks, events, habits, focus sessions, projects, settings,
  vault items): last-writer-wins **per field** by HLC; deletes are
  tombstones kept 90 days.
- **Note bodies:** Loro CRDT document per note. The editor still works on
  Markdown text; on save the client diffs old/new text into Loro operations,
  so concurrent offline edits merge instead of overwriting. Encrypted Loro
  updates are compacted into snapshots by the client (the server can't read
  them).
- **Server data model:** `changes(account_id, seq bigserial, device_id,
  collection, item_id, blob, created_at)`; clients push batches and pull
  `seq > cursor`. Idempotent push (client change IDs), size limits per blob
  (1 MiB) and per batch.
- **Real time:** a WebSocket per device receives "seq advanced" notices and
  pulls; offline devices catch up on reconnect. Fallback: poll every 60 s.
- **Search** stays local (FTS5 on the device); the server has no index.
- **Quotas:** Cloud includes 5 GB; history 30 days; the server reports usage
  in the app and admin panel. Attachments (later) as encrypted 4 MiB chunks in
  object storage.
- **First sync** on an existing install uploads everything encrypted with a
  progress bar; a second device downloads it, then both stay live.

## 5. Plans and billing

| | Free | Ottero Cloud |
| --- | --- | --- |
| Every feature on one computer | yes | yes |
| Account | no | yes |
| Encrypted sync, real time | — | yes |
| Encrypted cloud backup, 30-day note history | — | yes |
| QR device approval | — | yes |

- **Entitlements** are provider-agnostic on the server (`paid` / `grant` /
  `owner`); the sync server checks the user's active entitlement before
  accepting sync traffic.
- **Billing:** Upgrading or managing a subscription opens a hosted checkout or
  billing portal URL returned by the account API (`POST /api/billing/checkout`
  and `POST /api/billing/portal`). Local data is never locked behind a
  subscription.

## 6. Security checklist (gate before launch)

- [x] Threat model reviewed: stolen server DB, malicious server, stolen
      device, phishing, lost password, compromised admin account.
- [x] Crypto test vectors, round trips and tamper tests in `crates/crypto`.
- [x] Two-step verification (TOTP) supported in the app.
- [x] Secrets kept strictly in server environment variables; none in repos or
      logs; logs never contain verification codes, tokens, keys, or payloads.
- [x] Webhook signatures verified; replayed events ignored.
- [x] Privacy policy and terms published before accepting paid sign-ups.

## 7. Steps

1. [x] This plan + ADR 0010 accepted.
2. [x] Hosted account API and sync server deployed (private backend repo).
3. [x] App: Settings → Account (sign up, verify, sign in, sign out), key
       derivation and recovery kit in `crates/crypto`; tokens per keychain policy.
   - [x] `ottero_crypto::account`: Argon2id + Secret Key + HKDF, wrapped
         account key, recovery code; fixed vectors (ADR 0011).
   - [x] `ottero_core::account`: email/password rules, state machine.
   - [x] `apps/desktop/src/account/`: HTTP client, `account.json` (0600),
         `Library` methods, Emergency Kit (printable HTML).
   - [x] Settings → Account: intro, create account, code, Emergency Kit,
         sign in (+ Secret Key on a new device), signed-in page with
         plan, Secret Key, kit, devices and sign-out; actions + palette.
   - [x] Tests: crypto/core units, client against an in-process server,
         UI flows.
   - [x] Two-step verification (2026-11-17): sign-in asks for the
         authenticator or a backup code; Security → Two-step verification
         turns it on, gives new backup codes, turns it off. QR via `qrcode`
         (ADR 0013).
   - [x] Passwords under the account key (2026-11-18, ADR 0014): link an
         existing vault with its master password once, new vaults set up with
         no master password, one unlock opens account and Passwords, auto-lock
         and sign-out lock both, backups of linked vaults are archive v2.
   - [ ] Later: release builds keep token + Secret Key in the OS keychain;
         password reset with the recovery code; QR device approval.
         Account-key rotation must re-wrap the linked vault key.
4. [x] `crates/sync-protocol` and desktop E2EE sync engine (`apps/desktop/src/sync/`,
       outbox migration `0013_sync.sql`, HLC, per-field LWW, `SyncNow` action).
5. [ ] Background sync scheduler + WebSocket live push listener + multi-device
       account-wrapped `vault_key` sync record.
6. [ ] QR device approval, key rotation on device removal, account deletion.
