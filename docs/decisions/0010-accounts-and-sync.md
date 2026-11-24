# 0010 Accounts, encryption and sync

Status: accepted (2026-11-17).

## Context

Ottero Cloud adds paid, optional sync. The owner wants a custom sign-up/sign-in
inside the app (no Google/Apple sign-in for now), a web admin panel to manage
users and plans, Dodo Payments billing (Merchant of Record) and Brevo email. Data must stay end-to-end
encrypted. Plan: `docs/plans/06-sync.md`.

## Decision

1. **Better Auth** (MIT, TypeScript) inside our own account API, with
   PostgreSQL, rather than Zitadel. Zitadel is a full identity product with its
   own login pages and console; we need our own screens, our own admin panel
   and entitlements next to billing, which Better Auth's plugins (admin, email
   OTP, two-factor, passkey, JWT/JWKS) support directly. Impersonation is
   disabled.
2. **One password plus a Secret Key** (1Password model): the account password
   signs in and unlocks everything, including the vault section; there is no
   separate vault password for Cloud users. Argon2id on the device, mixed
   with a 128-bit Secret Key the server never sees, then HKDF into an auth key
   (sent, then hashed again by the server) and a wrap key (never sent). A new
   device gets the Secret Key once (QR approval or the Emergency Kit); unlock
   afterwards is the password or Touch ID / Windows Hello. A **recovery code**
   in the Emergency Kit covers a forgotten password.
3. **Random account key** wrapped by the password, the recovery key and each
   device (HPKE); per-collection keys under it; XChaCha20-Poly1305 per item.
4. **Own Rust sync server** (axum + PostgreSQL): per-field last-writer-wins by
   hybrid logical clock for records; Loro CRDT for note bodies; WebSocket
   change notices. Hosted sync engines that need plaintext are excluded.
5. **Entitlements** in one provider-agnostic table checked by the sync server;
   Dodo Payments webhooks, owner grants and the owner account all write to it.

## Consequences

- Losing both the password and the Emergency Kit means the cloud copy can
  only be reset, not recovered. The app must say so at sign-up.
- A stolen server database can't be brute-forced offline, because the Secret
  Key is never on the server.
- The account API is a second runtime (Node) next to the Rust sync server.
- Better Auth stores the password verifier format we choose (the auth key);
  its built-in password hashing is applied to the auth key, never the password.
- Search and any server-side features can't read content.
