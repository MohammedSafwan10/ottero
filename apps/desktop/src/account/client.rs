//! The Ottero Cloud account API over HTTPS (ureq, rustls), behind the
//! [`AccountApi`] trait so tests can swap in [`super::fake::FakeAccountApi`].
//!
//! Calls are blocking: run them on the background executor. Nothing here
//! logs. Request bodies (login keys, codes) and tokens live in zeroizing
//! buffers and are never part of an error.

use std::time::Duration;

use jiff::Timestamp;
use ottero_crypto::account::{AccountKdf, decode_salt};
use ottero_crypto::{Salt, Zeroizing};
use serde::Deserialize;
use serde_json::json;

/// Production account server.
pub const PRODUCTION_URL: &str = "https://account.nexdark.com";
/// The local development API (`ottero-cloud/api/dev.sh`).
pub const DEVELOPMENT_URL: &str = "http://127.0.0.1:8787";
/// Overrides the server address.
pub const URL_VAR: &str = "OTTERO_ACCOUNT_URL";

const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;

/// What went wrong, in terms the account screens can explain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// No connection, DNS failure, timeout or TLS failure.
    Offline,
    WrongCode,
    CodeExpired,
    /// Wrong password or Secret Key (the server can't tell which).
    WrongCredentials,
    NotVerified,
    TooManyAttempts,
    /// Blocked by Ottero; the server's message says whom to contact.
    Blocked(String),
    /// The session token is no longer accepted.
    SessionEnded,
    /// `POST /api/keys` when keys exist already.
    KeysExist,
    /// The server refused the request as invalid.
    Rejected,
    /// 5xx.
    Server,
    /// The response wasn't what this version of Ottero understands.
    Unexpected,
    /// `OTTERO_ACCOUNT_URL` is unusable (for example not https in a release build).
    Misconfigured,
    /// The authenticator code was wrong (sign-in or turning two-step on).
    WrongTwoStepCode,
    /// The backup code was wrong or already used.
    WrongBackupCode,
    /// Too many wrong codes across sign-ins: the server locks two-step
    /// verification for a while (15 minutes).
    TwoStepLocked,
    /// The second step of this sign-in is over (it expired after 10 minutes,
    /// or two-step verification was turned off meanwhile): enter the password again.
    TwoStepExpired,
    /// Too many wrong codes in this sign-in: enter the password again.
    TwoStepTooManyCodes,
    /// Two-step verification was turned on or off elsewhere in the meantime.
    TwoStepChanged,
}

impl ApiError {
    /// A calm, complete sentence for the user.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Offline => "Couldn't reach Ottero Cloud. Check your connection and try again.".into(),
            Self::WrongCode => "That code isn't right. Check the email and try again.".into(),
            Self::CodeExpired => "That code has expired. Send a new one.".into(),
            Self::WrongCredentials => "Wrong email, password or Secret Key.".into(),
            Self::NotVerified => "Confirm your email first.".into(),
            Self::TooManyAttempts => "Too many attempts. Wait a minute and try again.".into(),
            Self::Blocked(message) => message.clone(),
            Self::SessionEnded => "You were signed out of Ottero Cloud. Sign in again.".into(),
            Self::KeysExist => "This account's keys are already set up.".into(),
            Self::Rejected => "Ottero Cloud didn't accept that. Update Ottero and try again.".into(),
            Self::Server => "Ottero Cloud had a problem. Try again in a few minutes.".into(),
            Self::Unexpected => "Ottero Cloud sent an answer Ottero doesn't understand. Update Ottero and try again.".into(),
            Self::Misconfigured => "The Ottero Cloud address isn't secure, so Ottero won't use it.".into(),
            Self::WrongTwoStepCode => "That code isn't right. Check your authenticator app and try again.".into(),
            Self::WrongBackupCode => "That backup code isn't right, or it was already used.".into(),
            Self::TwoStepLocked => "Too many wrong codes. Try again in 15 minutes.".into(),
            Self::TwoStepExpired => "This sign-in timed out. Enter your password again.".into(),
            Self::TwoStepTooManyCodes => "Too many wrong codes. Enter your password again.".into(),
            Self::TwoStepChanged => "Two-step verification was changed on another device. Close this and try again.".into(),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// The bearer token for one signed-in device. Never printed.
pub struct SessionToken(Zeroizing<String>);

impl SessionToken {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The token without its signature, as `list-sessions` reports it.
    #[must_use]
    pub fn session_part(&self) -> &str {
        self.0.split('.').next().unwrap_or_default()
    }
}

impl Clone for SessionToken {
    fn clone(&self) -> Self {
        Self::new(self.0.to_string())
    }
}

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken([redacted])")
    }
}

/// The second step of a sign-in with two-step verification on: the signed
/// `two_factor` cookie from `sign-in/email` (`name=value`). It proves the
/// password was right for about 10 minutes. Held in memory only, never
/// written to disk and never printed.
pub struct TwoStepChallenge(Zeroizing<String>);

impl TwoStepChallenge {
    #[must_use]
    pub fn new(cookie: String) -> Self {
        Self(Zeroizing::new(cookie))
    }

    /// The `Cookie` header value.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for TwoStepChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TwoStepChallenge([redacted])")
    }
}

/// What `sign-in/email` answered for a right password.
#[derive(Debug)]
pub enum SignInOutcome {
    Session(SessionToken),
    /// Two-step verification is on: no session yet, a code comes next.
    TwoStep(TwoStepChallenge),
}

/// Which code finishes a two-step sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondFactor {
    /// The 6-digit code from the authenticator app.
    Authenticator,
    /// One of the ten single-use backup codes.
    BackupCode,
}

/// `two-factor/enable`: the authenticator secret as an `otpauth://` link
/// (for the QR code), its base32 setup key, and the backup codes. Not active
/// until a code from the app confirms it.
pub struct TwoStepSetup {
    uri: Zeroizing<String>,
    secret: Zeroizing<String>,
    backup_codes: Vec<Zeroizing<String>>,
}

impl TwoStepSetup {
    /// Checks the link is a TOTP link with a base32 secret.
    pub fn new(uri: String, backup_codes: Vec<String>) -> ApiResult<Self> {
        let uri = Zeroizing::new(uri);
        let secret = setup_key_of(&uri).ok_or(ApiError::Unexpected)?;
        if backup_codes.is_empty() {
            return Err(ApiError::Unexpected);
        }
        Ok(Self {
            uri,
            secret,
            backup_codes: backup_codes.into_iter().map(Zeroizing::new).collect(),
        })
    }

    #[must_use]
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// The base32 secret, for typing into an app that can't scan.
    #[must_use]
    pub fn setup_key(&self) -> &str {
        &self.secret
    }

    #[must_use]
    pub fn backup_codes(&self) -> &[Zeroizing<String>] {
        &self.backup_codes
    }
}

/// The `secret` of an `otpauth://totp/…` link, upper case, if it is base32.
#[must_use]
pub fn setup_key_of(uri: &str) -> Option<Zeroizing<String>> {
    let parsed = url::Url::parse(uri).ok()?;
    if parsed.scheme() != "otpauth" || parsed.host_str() != Some("totp") {
        return None;
    }
    let secret = parsed
        .query_pairs()
        .find(|(k, _)| k == "secret")
        .map(|(_, v)| Zeroizing::new(v.trim_end_matches('=').to_uppercase()))?;
    let base32 = |c: char| c.is_ascii_uppercase() || ('2'..='7').contains(&c);
    (secret.len() >= 16 && secret.chars().all(base32)).then_some(secret)
}

/// The public salt and KDF settings for an email (`POST /api/kdf`).
pub struct KdfInfo {
    pub salt: Salt,
    pub kdf: AccountKdf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cloud {
    pub source: String,
    pub status: String,
    pub ends_at: Option<Timestamp>,
}

/// `GET /api/me`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub id: String,
    pub email: String,
    pub role: String,
    pub cloud: Option<Cloud>,
    pub has_keys: bool,
    /// Two-step verification is on.
    pub two_factor: bool,
}

impl Me {
    #[must_use]
    pub fn plan(&self) -> ottero_core::account::Plan {
        ottero_core::account::Plan::from_parts(
            &self.role,
            self.cloud.as_ref().map(|c| (c.source.as_str(), c.ends_at)),
        )
    }
}

/// The account key, wrapped twice (opaque to the server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedKeys {
    pub by_password: String,
    pub by_recovery: String,
}

/// One signed-in device, from `list-sessions`.
pub struct SessionInfo {
    pub id: String,
    /// Needed to revoke it. Never shown.
    token: Zeroizing<String>,
    pub user_agent: Option<String>,
    pub created_at: Option<Timestamp>,
    pub last_active: Option<Timestamp>,
}

impl SessionInfo {
    #[must_use]
    pub fn new(
        id: String,
        token: String,
        user_agent: Option<String>,
        created_at: Option<Timestamp>,
        last_active: Option<Timestamp>,
    ) -> Self {
        Self {
            id,
            token: Zeroizing::new(token),
            user_agent,
            created_at,
            last_active,
        }
    }

    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    #[must_use]
    pub fn is_current(&self, current: &SessionToken) -> bool {
        self.token.as_str() == current.session_part()
    }
}

/// The account server. Every method blocks; call from a background thread.
pub trait AccountApi: Send + Sync {
    fn kdf(&self, email: &str) -> ApiResult<KdfInfo>;
    /// Creates an unverified account; the server emails a code. The same
    /// answer comes back for an email that already has an account.
    fn sign_up(&self, email: &str, name: &str, auth_key_hex: &str, salt: &str) -> ApiResult<()>;
    fn verify_email(&self, email: &str, code: &str) -> ApiResult<()>;
    fn resend_code(&self, email: &str) -> ApiResult<()>;
    /// A session, or the challenge for the second step when two-step
    /// verification is on.
    fn sign_in(&self, email: &str, auth_key_hex: &str) -> ApiResult<SignInOutcome>;
    /// Finishes a two-step sign-in with an authenticator or backup code.
    fn verify_second_factor(
        &self,
        challenge: &TwoStepChallenge,
        factor: SecondFactor,
        code: &str,
    ) -> ApiResult<SessionToken>;
    /// Starts turning two-step verification on (inactive until confirmed).
    fn enable_two_factor(
        &self,
        token: &SessionToken,
        auth_key_hex: &str,
    ) -> ApiResult<TwoStepSetup>;
    /// Confirms the setup with a code from the app. The server replaces the
    /// session; the replacement token comes back (`None`: keep the old one).
    fn confirm_two_factor(
        &self,
        token: &SessionToken,
        code: &str,
    ) -> ApiResult<Option<SessionToken>>;
    /// Turns two-step verification off; the session is replaced as above.
    fn disable_two_factor(
        &self,
        token: &SessionToken,
        auth_key_hex: &str,
    ) -> ApiResult<Option<SessionToken>>;
    /// Replaces every backup code with a new set.
    fn regenerate_backup_codes(
        &self,
        token: &SessionToken,
        auth_key_hex: &str,
    ) -> ApiResult<Vec<Zeroizing<String>>>;
    fn me(&self, token: &SessionToken) -> ApiResult<Me>;
    fn put_keys(&self, token: &SessionToken, keys: &WrappedKeys) -> ApiResult<()>;
    /// `None` when the account has no keys yet.
    fn get_keys(&self, token: &SessionToken) -> ApiResult<Option<WrappedKeys>>;
    fn sign_out(&self, token: &SessionToken) -> ApiResult<()>;
    fn sessions(&self, token: &SessionToken) -> ApiResult<Vec<SessionInfo>>;
    fn revoke_session(&self, token: &SessionToken, session_token: &str) -> ApiResult<()>;
    /// Creates a hosted checkout URL for `"monthly"` or `"yearly"`.
    fn billing_checkout(&self, _token: &SessionToken, _interval: &str) -> ApiResult<String> {
        Err(ApiError::Server)
    }
    /// Creates a hosted customer billing portal URL.
    fn billing_portal(&self, _token: &SessionToken) -> ApiResult<String> {
        Err(ApiError::Server)
    }
}

/// The server address: `OTTERO_ACCOUNT_URL`, else the build's default.
/// Release builds accept only `https://`.
pub fn base_url(value: Option<&str>, release: bool) -> ApiResult<String> {
    let default = if release {
        PRODUCTION_URL
    } else {
        DEVELOPMENT_URL
    };
    let url = value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(default);
    let parsed = url::Url::parse(url).map_err(|_| ApiError::Misconfigured)?;
    let allowed = match parsed.scheme() {
        "https" => true,
        "http" => !release,
        _ => false,
    };
    if !allowed || parsed.host_str().is_none() || !parsed.username().is_empty() {
        return Err(ApiError::Misconfigured);
    }
    Ok(url.trim_end_matches('/').to_owned())
}

/// Used when the address is unusable: every call fails with `Misconfigured`.
pub struct Unconfigured;

impl AccountApi for Unconfigured {
    fn kdf(&self, _: &str) -> ApiResult<KdfInfo> {
        Err(ApiError::Misconfigured)
    }
    fn sign_up(&self, _: &str, _: &str, _: &str, _: &str) -> ApiResult<()> {
        Err(ApiError::Misconfigured)
    }
    fn verify_email(&self, _: &str, _: &str) -> ApiResult<()> {
        Err(ApiError::Misconfigured)
    }
    fn resend_code(&self, _: &str) -> ApiResult<()> {
        Err(ApiError::Misconfigured)
    }
    fn sign_in(&self, _: &str, _: &str) -> ApiResult<SignInOutcome> {
        Err(ApiError::Misconfigured)
    }
    fn verify_second_factor(
        &self,
        _: &TwoStepChallenge,
        _: SecondFactor,
        _: &str,
    ) -> ApiResult<SessionToken> {
        Err(ApiError::Misconfigured)
    }
    fn enable_two_factor(&self, _: &SessionToken, _: &str) -> ApiResult<TwoStepSetup> {
        Err(ApiError::Misconfigured)
    }
    fn confirm_two_factor(&self, _: &SessionToken, _: &str) -> ApiResult<Option<SessionToken>> {
        Err(ApiError::Misconfigured)
    }
    fn disable_two_factor(&self, _: &SessionToken, _: &str) -> ApiResult<Option<SessionToken>> {
        Err(ApiError::Misconfigured)
    }
    fn regenerate_backup_codes(
        &self,
        _: &SessionToken,
        _: &str,
    ) -> ApiResult<Vec<Zeroizing<String>>> {
        Err(ApiError::Misconfigured)
    }
    fn me(&self, _: &SessionToken) -> ApiResult<Me> {
        Err(ApiError::Misconfigured)
    }
    fn put_keys(&self, _: &SessionToken, _: &WrappedKeys) -> ApiResult<()> {
        Err(ApiError::Misconfigured)
    }
    fn get_keys(&self, _: &SessionToken) -> ApiResult<Option<WrappedKeys>> {
        Err(ApiError::Misconfigured)
    }
    fn sign_out(&self, _: &SessionToken) -> ApiResult<()> {
        Err(ApiError::Misconfigured)
    }
    fn sessions(&self, _: &SessionToken) -> ApiResult<Vec<SessionInfo>> {
        Err(ApiError::Misconfigured)
    }
    fn revoke_session(&self, _: &SessionToken, _: &str) -> ApiResult<()> {
        Err(ApiError::Misconfigured)
    }
}

pub struct HttpAccountClient {
    base: String,
    /// `scheme://host[:port]` of `base`. Better Auth refuses a request that
    /// carries a cookie without a trusted `Origin` (CSRF check); the server's
    /// own address is trusted, so the two-step cookie request names it.
    origin: String,
    agent: ureq::Agent,
}

struct Reply {
    status: u16,
    body: Zeroizing<Vec<u8>>,
    auth_token: Option<SessionToken>,
    /// The `two_factor` cookie of a sign-in that needs the second step.
    two_step: Option<TwoStepChallenge>,
}

/// How a request proves who sends it.
#[derive(Clone, Copy)]
enum Credential<'a> {
    None,
    Bearer(&'a SessionToken),
    /// The two-step challenge cookie, for the second step of a sign-in.
    Challenge(&'a TwoStepChallenge),
}

#[derive(Deserialize, Default)]
struct ErrorBody {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

impl HttpAccountClient {
    /// `base` must come from [`base_url`].
    #[must_use]
    pub fn new(base: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // Never follow a redirect: a login key must go only where it was sent.
            .max_redirects(0)
            .https_only(base.starts_with("https://"))
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(user_agent())
            .build()
            .into();
        let origin = url::Url::parse(&base)
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default();
        Self {
            base,
            origin,
            agent,
        }
    }

    /// From `OTTERO_ACCOUNT_URL` and the build type.
    #[must_use]
    pub fn from_env() -> Box<dyn AccountApi> {
        match base_url(
            std::env::var(URL_VAR).ok().as_deref(),
            !cfg!(debug_assertions),
        ) {
            Ok(base) => Box::new(Self::new(base)),
            Err(_) => {
                eprintln!("ottero: {URL_VAR} is not a usable https address; Ottero Cloud is off");
                Box::new(Unconfigured)
            }
        }
    }

    fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
        token: Option<&SessionToken>,
    ) -> ApiResult<Reply> {
        self.send(
            path,
            body,
            token.map_or(Credential::None, Credential::Bearer),
        )
    }

    fn send(
        &self,
        path: &str,
        body: &serde_json::Value,
        credential: Credential<'_>,
    ) -> ApiResult<Reply> {
        let bytes = Zeroizing::new(serde_json::to_vec(body).map_err(|_| ApiError::Unexpected)?);
        let mut request = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("content-type", "application/json");
        match credential {
            Credential::None => {}
            Credential::Bearer(token) => {
                request = request.header("authorization", format!("Bearer {}", token.expose()));
            }
            Credential::Challenge(challenge) => {
                request = request
                    .header("cookie", challenge.expose())
                    .header("origin", self.origin.as_str());
            }
        }
        read(request.send(bytes.as_slice()))
    }

    fn get(&self, path: &str, token: &SessionToken) -> ApiResult<Reply> {
        read(
            self.agent
                .get(format!("{}{path}", self.base))
                .header("authorization", format!("Bearer {}", token.expose()))
                .call(),
        )
    }
}

/// `Ottero/0.1.3 (macOS)`: names the device in the sessions list.
#[must_use]
pub fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    format!("Ottero/{} ({os})", env!("CARGO_PKG_VERSION"))
}

fn read(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> ApiResult<Reply> {
    let mut response = result.map_err(|err| match err {
        ureq::Error::BodyExceedsLimit(_)
        | ureq::Error::TooManyRedirects
        | ureq::Error::RedirectFailed => ApiError::Unexpected,
        _ => ApiError::Offline,
    })?;
    let status = response.status().as_u16();
    let auth_token = response
        .headers()
        .get("set-auth-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .map(|v| SessionToken::new(v.to_owned()));
    let two_step = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(two_step_cookie);
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_vec()
        .map_err(|_| ApiError::Offline)?;
    Ok(Reply {
        status,
        body: Zeroizing::new(body),
        auth_token,
        two_step,
    })
}

/// `name=value` of a `Set-Cookie` line for the two-step cookie
/// (`better-auth.two_factor`, or `__Secure-` prefixed over https). An
/// emptied cookie (the server expiring it) doesn't count.
fn two_step_cookie(line: &str) -> Option<TwoStepChallenge> {
    let pair = line.split(';').next()?.trim();
    let (name, value) = pair.split_once('=')?;
    let ours = name
        .trim_start_matches("__Secure-")
        .trim_start_matches("__Host-")
        == "better-auth.two_factor";
    (ours && !value.is_empty()).then(|| TwoStepChallenge::new(pair.to_owned()))
}

/// Maps a failed two-step reply. `session` is true while signed in
/// (turning it on), where any other 401 means the session ended.
fn two_step_error(reply: &Reply, session: bool) -> ApiError {
    let body = error_of(reply);
    match (reply.status, body.code.as_deref()) {
        (_, Some("INVALID_CODE")) => ApiError::WrongTwoStepCode,
        (_, Some("INVALID_BACKUP_CODE")) => ApiError::WrongBackupCode,
        (_, Some("ACCOUNT_TEMPORARILY_LOCKED")) => ApiError::TwoStepLocked,
        (_, Some("TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE")) => ApiError::TwoStepTooManyCodes,
        (_, Some("INVALID_PASSWORD")) => ApiError::WrongCredentials,
        (
            _,
            Some(
                "TOTP_ALREADY_ENABLED"
                | "TOTP_NOT_ENABLED"
                | "TWO_FACTOR_NOT_ENABLED"
                | "BACKUP_CODES_NOT_ENABLED",
            ),
        ) if session => ApiError::TwoStepChanged,
        (_, Some("TOTP_NOT_ENABLED" | "TWO_FACTOR_NOT_ENABLED" | "BACKUP_CODES_NOT_ENABLED")) => {
            ApiError::TwoStepExpired
        }
        (401, _) if session => ApiError::SessionEnded,
        (401, _) => ApiError::TwoStepExpired,
        _ => general(reply, session),
    }
}

/// `{ backupCodes: [...] }` from enable or generate-backup-codes.
fn backup_codes_of(reply: &Reply) -> ApiResult<Vec<String>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Body {
        backup_codes: Vec<String>,
    }
    let body: Body = parse(reply)?;
    if body.backup_codes.is_empty() || body.backup_codes.iter().any(|c| c.len() > 64) {
        return Err(ApiError::Unexpected);
    }
    Ok(body.backup_codes)
}

fn error_of(reply: &Reply) -> ErrorBody {
    serde_json::from_slice(&reply.body).unwrap_or_default()
}

/// Maps a failed reply for calls that need no special handling.
fn general(reply: &Reply, authed: bool) -> ApiError {
    let body = error_of(reply);
    match reply.status {
        401 if authed => ApiError::SessionEnded,
        401 => ApiError::WrongCredentials,
        429 => ApiError::TooManyAttempts,
        403 if body.code.as_deref() == Some("BANNED_USER")
            || body
                .message
                .as_deref()
                .is_some_and(|m| m.contains("blocked")) =>
        {
            ApiError::Blocked(blocked_message(&body))
        }
        400..=499 => ApiError::Rejected,
        500..=599 => ApiError::Server,
        _ => ApiError::Unexpected,
    }
}

fn blocked_message(body: &ErrorBody) -> String {
    // The server's own sentence names the contact; keep it short and plain.
    body.message
        .as_deref()
        .filter(|m| m.len() <= 200 && m.contains("blocked"))
        .map_or_else(|| "This account is blocked.".to_owned(), str::to_owned)
}

fn ok(reply: &Reply) -> bool {
    (200..300).contains(&reply.status)
}

fn parse<T: for<'de> Deserialize<'de>>(reply: &Reply) -> ApiResult<T> {
    serde_json::from_slice(&reply.body).map_err(|_| ApiError::Unexpected)
}

fn timestamp(value: Option<&str>) -> Option<Timestamp> {
    value.and_then(|v| v.parse().ok())
}

impl AccountApi for HttpAccountClient {
    fn kdf(&self, email: &str) -> ApiResult<KdfInfo> {
        #[derive(Deserialize)]
        struct Body {
            salt: String,
            kdf: String,
        }
        let reply = self.post("/api/kdf", &json!({ "email": email }), None)?;
        if !ok(&reply) {
            return Err(general(&reply, false));
        }
        let body: Body = parse(&reply)?;
        Ok(KdfInfo {
            salt: decode_salt(&body.salt).map_err(|_| ApiError::Unexpected)?,
            kdf: AccountKdf::parse(&body.kdf).map_err(|_| ApiError::Unexpected)?,
        })
    }

    fn sign_up(&self, email: &str, name: &str, auth_key_hex: &str, salt: &str) -> ApiResult<()> {
        let body = json!({
            "email": email,
            "name": name,
            "password": auth_key_hex,
            "kdf": { "salt": salt, "params": ottero_crypto::account::KDF_V1 },
        });
        let reply = self.post("/api/auth/sign-up/email", &body, None)?;
        if ok(&reply) {
            Ok(())
        } else {
            Err(general(&reply, false))
        }
    }

    fn verify_email(&self, email: &str, code: &str) -> ApiResult<()> {
        let reply = self.post(
            "/api/auth/email-otp/verify-email",
            &json!({ "email": email, "otp": code }),
            None,
        )?;
        if ok(&reply) {
            return Ok(());
        }
        let body = error_of(&reply);
        Err(match (reply.status, body.code.as_deref()) {
            (_, Some("OTP_EXPIRED")) => ApiError::CodeExpired,
            (_, Some("TOO_MANY_ATTEMPTS")) | (429, _) => ApiError::TooManyAttempts,
            (400, _) => ApiError::WrongCode,
            _ => general(&reply, false),
        })
    }

    fn resend_code(&self, email: &str) -> ApiResult<()> {
        let reply = self.post(
            "/api/auth/email-otp/send-verification-otp",
            &json!({ "email": email, "type": "email-verification" }),
            None,
        )?;
        if ok(&reply) {
            Ok(())
        } else {
            Err(general(&reply, false))
        }
    }

    fn sign_in(&self, email: &str, auth_key_hex: &str) -> ApiResult<SignInOutcome> {
        #[derive(Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            #[serde(default)]
            two_factor_redirect: bool,
        }
        let reply = self.post(
            "/api/auth/sign-in/email",
            &json!({ "email": email, "password": auth_key_hex }),
            None,
        )?;
        if ok(&reply) {
            let body: Body = serde_json::from_slice(&reply.body).unwrap_or_default();
            if body.two_factor_redirect {
                // Any token header on this answer is not a session: ignore it.
                return reply
                    .two_step
                    .map(SignInOutcome::TwoStep)
                    .ok_or(ApiError::Unexpected);
            }
            return reply
                .auth_token
                .map(SignInOutcome::Session)
                .ok_or(ApiError::Unexpected);
        }
        let body = error_of(&reply);
        Err(match (reply.status, body.code.as_deref()) {
            (403, Some("EMAIL_NOT_VERIFIED")) => ApiError::NotVerified,
            (401, _) => ApiError::WrongCredentials,
            _ => general(&reply, false),
        })
    }

    fn verify_second_factor(
        &self,
        challenge: &TwoStepChallenge,
        factor: SecondFactor,
        code: &str,
    ) -> ApiResult<SessionToken> {
        let path = match factor {
            SecondFactor::Authenticator => "/api/auth/two-factor/verify-totp",
            SecondFactor::BackupCode => "/api/auth/two-factor/verify-backup-code",
        };
        let reply = self.send(
            path,
            &json!({ "code": code }),
            Credential::Challenge(challenge),
        )?;
        if ok(&reply) {
            return reply.auth_token.ok_or(ApiError::Unexpected);
        }
        Err(two_step_error(&reply, false))
    }

    fn enable_two_factor(
        &self,
        token: &SessionToken,
        auth_key_hex: &str,
    ) -> ApiResult<TwoStepSetup> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            #[serde(rename = "totpURI")]
            totp_uri: String,
        }
        let reply = self.post(
            "/api/auth/two-factor/enable",
            &json!({ "password": auth_key_hex }),
            Some(token),
        )?;
        if !ok(&reply) {
            return Err(two_step_error(&reply, true));
        }
        let body: Body = parse(&reply)?;
        TwoStepSetup::new(body.totp_uri, backup_codes_of(&reply)?)
    }

    fn confirm_two_factor(
        &self,
        token: &SessionToken,
        code: &str,
    ) -> ApiResult<Option<SessionToken>> {
        let reply = self.post(
            "/api/auth/two-factor/verify-totp",
            &json!({ "code": code }),
            Some(token),
        )?;
        if ok(&reply) {
            Ok(reply.auth_token)
        } else {
            Err(two_step_error(&reply, true))
        }
    }

    fn disable_two_factor(
        &self,
        token: &SessionToken,
        auth_key_hex: &str,
    ) -> ApiResult<Option<SessionToken>> {
        let reply = self.post(
            "/api/auth/two-factor/disable",
            &json!({ "password": auth_key_hex }),
            Some(token),
        )?;
        if ok(&reply) {
            Ok(reply.auth_token)
        } else {
            Err(two_step_error(&reply, true))
        }
    }

    fn regenerate_backup_codes(
        &self,
        token: &SessionToken,
        auth_key_hex: &str,
    ) -> ApiResult<Vec<Zeroizing<String>>> {
        let reply = self.post(
            "/api/auth/two-factor/generate-backup-codes",
            &json!({ "password": auth_key_hex }),
            Some(token),
        )?;
        if !ok(&reply) {
            return Err(two_step_error(&reply, true));
        }
        Ok(backup_codes_of(&reply)?
            .into_iter()
            .map(Zeroizing::new)
            .collect())
    }

    fn me(&self, token: &SessionToken) -> ApiResult<Me> {
        #[derive(Deserialize)]
        struct CloudBody {
            source: String,
            status: String,
            ends_at: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            id: String,
            email: String,
            role: String,
            cloud: Option<CloudBody>,
            has_keys: bool,
            #[serde(default)]
            two_factor: bool,
        }
        let reply = self.get("/api/me", token)?;
        if !ok(&reply) {
            return Err(general(&reply, true));
        }
        let body: Body = parse(&reply)?;
        Ok(Me {
            id: body.id,
            email: body.email,
            role: body.role,
            cloud: body.cloud.map(|c| Cloud {
                source: c.source,
                status: c.status,
                ends_at: timestamp(c.ends_at.as_deref()),
            }),
            has_keys: body.has_keys,
            two_factor: body.two_factor,
        })
    }

    fn put_keys(&self, token: &SessionToken, keys: &WrappedKeys) -> ApiResult<()> {
        let reply = self.post(
            "/api/keys",
            &json!({ "wrapped_by_password": keys.by_password, "wrapped_by_recovery": keys.by_recovery }),
            Some(token),
        )?;
        match reply.status {
            201 | 200 => Ok(()),
            409 => Err(ApiError::KeysExist),
            _ => Err(general(&reply, true)),
        }
    }

    fn get_keys(&self, token: &SessionToken) -> ApiResult<Option<WrappedKeys>> {
        #[derive(Deserialize)]
        struct Body {
            wrapped_by_password: String,
            wrapped_by_recovery: String,
        }
        let reply = self.get("/api/keys", token)?;
        match reply.status {
            404 => Ok(None),
            s if (200..300).contains(&s) => {
                let body: Body = parse(&reply)?;
                Ok(Some(WrappedKeys {
                    by_password: body.wrapped_by_password,
                    by_recovery: body.wrapped_by_recovery,
                }))
            }
            _ => Err(general(&reply, true)),
        }
    }

    fn sign_out(&self, token: &SessionToken) -> ApiResult<()> {
        let reply = self.post("/api/auth/sign-out", &json!({}), Some(token))?;
        // Already signed out on the server counts as done.
        if ok(&reply) || reply.status == 401 {
            Ok(())
        } else {
            Err(general(&reply, true))
        }
    }

    fn sessions(&self, token: &SessionToken) -> ApiResult<Vec<SessionInfo>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            id: String,
            token: String,
            user_agent: Option<String>,
            created_at: Option<String>,
            updated_at: Option<String>,
        }
        let reply = self.get("/api/auth/list-sessions", token)?;
        if !ok(&reply) {
            return Err(general(&reply, true));
        }
        let rows: Vec<Body> = parse(&reply)?;
        Ok(rows
            .into_iter()
            .map(|r| {
                SessionInfo::new(
                    r.id,
                    r.token,
                    r.user_agent,
                    timestamp(r.created_at.as_deref()),
                    timestamp(r.updated_at.as_deref()),
                )
            })
            .collect())
    }

    fn revoke_session(&self, token: &SessionToken, session_token: &str) -> ApiResult<()> {
        let reply = self.post(
            "/api/auth/revoke-session",
            &json!({ "token": session_token }),
            Some(token),
        )?;
        if ok(&reply) {
            Ok(())
        } else {
            Err(general(&reply, true))
        }
    }

    fn billing_checkout(&self, token: &SessionToken, interval: &str) -> ApiResult<String> {
        #[derive(Deserialize)]
        struct Body {
            url: String,
        }
        let reply = self.post(
            "/api/billing/checkout",
            &json!({ "interval": interval }),
            Some(token),
        )?;
        if !ok(&reply) {
            return Err(general(&reply, true));
        }
        let body: Body = parse(&reply)?;
        Ok(body.url)
    }

    fn billing_portal(&self, token: &SessionToken) -> ApiResult<String> {
        #[derive(Deserialize)]
        struct Body {
            url: String,
        }
        let reply = self.post("/api/billing/portal", &json!({}), Some(token))?;
        if !ok(&reply) {
            return Err(general(&reply, true));
        }
        let body: Body = parse(&reply)?;
        Ok(body.url)
    }
}

#[cfg(test)]
mod tests {
    //! Against a tiny in-process HTTP server that replays the real API's
    //! answers (captured from `ottero-cloud/api`), so these run in check.sh.
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct Seen {
        method: String,
        path: String,
        authorization: Option<String>,
        cookie: Option<String>,
        origin: Option<String>,
        body: String,
    }

    /// One canned answer: status, extra headers, body.
    type Canned = (u16, Vec<(&'static str, &'static str)>, &'static str);

    /// Serves `replies` in order and records the requests.
    fn server(replies: Vec<Canned>) -> (String, Arc<Mutex<Vec<Seen>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, headers, body) in replies {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("request line");
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_owned();
                let path = parts.next().unwrap_or_default().to_owned();
                let (mut length, mut authorization, mut cookie, mut origin) =
                    (0usize, None, None, None);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).expect("header");
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (name, value) = header.split_once(':').unwrap_or_default();
                    match name.to_ascii_lowercase().as_str() {
                        "content-length" => length = value.trim().parse().unwrap_or(0),
                        "authorization" => authorization = Some(value.trim().to_owned()),
                        "cookie" => cookie = Some(value.trim().to_owned()),
                        "origin" => origin = Some(value.trim().to_owned()),
                        _ => {}
                    }
                }
                let mut buf = vec![0; length];
                reader.read_exact(&mut buf).expect("body");
                log.lock().expect("log").push(Seen {
                    method,
                    path,
                    authorization,
                    cookie,
                    origin,
                    body: String::from_utf8_lossy(&buf).into_owned(),
                });
                let mut out = stream;
                let mut head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                _ = out.write_all(head.as_bytes());
                _ = out.write_all(body.as_bytes());
            }
        });
        (base, seen)
    }

    const HEX: &str = "ab";

    #[test]
    fn base_url_rules() {
        assert_eq!(base_url(None, false).as_deref(), Ok(DEVELOPMENT_URL));
        assert_eq!(base_url(None, true).as_deref(), Ok(PRODUCTION_URL));
        assert_eq!(
            base_url(Some("https://x.example/"), true).as_deref(),
            Ok("https://x.example")
        );
        assert_eq!(
            base_url(Some("http://127.0.0.1:9"), false).as_deref(),
            Ok("http://127.0.0.1:9")
        );
        assert_eq!(
            base_url(Some("http://account.nexdark.com"), true),
            Err(ApiError::Misconfigured)
        );
        assert_eq!(
            base_url(Some("ftp://x"), false),
            Err(ApiError::Misconfigured)
        );
        assert_eq!(
            base_url(Some("https://u:p@x.example"), true),
            Err(ApiError::Misconfigured)
        );
        assert_eq!(base_url(Some("  "), true).as_deref(), Ok(PRODUCTION_URL));
    }

    #[test]
    fn sign_in_reads_the_token_header_and_sends_the_key() {
        let (base, seen) = server(vec![
            (
                200,
                vec![("set-auth-token", "raw123.sig=")],
                r#"{"redirect":false,"token":"raw123"}"#,
            ),
            (
                403,
                vec![],
                r#"{"message":"Email not verified","code":"EMAIL_NOT_VERIFIED"}"#,
            ),
            (
                401,
                vec![],
                r#"{"message":"Invalid email or password","code":"INVALID_EMAIL_OR_PASSWORD"}"#,
            ),
            (429, vec![], r#"{"message":"Too many requests"}"#),
            (
                403,
                vec![],
                r#"{"message":"This account is blocked. Contact hello@nexdark.com.","code":"BANNED_USER"}"#,
            ),
            (500, vec![], r#"{"error":"Something went wrong"}"#),
        ]);
        let api = HttpAccountClient::new(base);
        let SignInOutcome::Session(token) = api.sign_in("sam@example.com", HEX).expect("token")
        else {
            panic!("a session");
        };
        assert_eq!(token.expose(), "raw123.sig=");
        assert_eq!(token.session_part(), "raw123");
        assert_eq!(format!("{token:?}"), "SessionToken([redacted])");
        assert_eq!(
            api.sign_in("a@b.co", HEX).map(|_| ()),
            Err(ApiError::NotVerified)
        );
        assert_eq!(
            api.sign_in("a@b.co", HEX).map(|_| ()),
            Err(ApiError::WrongCredentials)
        );
        assert_eq!(
            api.sign_in("a@b.co", HEX).map(|_| ()),
            Err(ApiError::TooManyAttempts)
        );
        assert_eq!(
            api.sign_in("a@b.co", HEX).map(|_| ()),
            Err(ApiError::Blocked(
                "This account is blocked. Contact hello@nexdark.com.".into()
            ))
        );
        assert_eq!(
            api.sign_in("a@b.co", HEX).map(|_| ()),
            Err(ApiError::Server)
        );
        let seen = seen.lock().expect("seen");
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/api/auth/sign-in/email");
        let body: serde_json::Value = serde_json::from_str(&seen[0].body).expect("json");
        assert_eq!(body, json!({ "email": "sam@example.com", "password": HEX }));
    }

    #[test]
    fn kdf_accepts_only_v1_and_valid_salts() {
        let (base, _) = server(vec![
            (
                200,
                vec![],
                r#"{"salt":"B465J6Y1apD8t4WQw10Jzg","kdf":"argon2id:m=262144,t=3,p=1:v1"}"#,
            ),
            (
                200,
                vec![],
                r#"{"salt":"B465J6Y1apD8t4WQw10Jzg","kdf":"argon2id:m=1024,t=1,p=1:v1"}"#,
            ),
            (
                200,
                vec![],
                r#"{"salt":"short","kdf":"argon2id:m=262144,t=3,p=1:v1"}"#,
            ),
            (400, vec![], r#"{"error":"Enter a valid email"}"#),
        ]);
        let api = HttpAccountClient::new(base);
        let info = api.kdf("sam@example.com").expect("kdf");
        assert_eq!(info.kdf, AccountKdf::V1);
        assert_eq!(
            api.kdf("x").map(|_| ()),
            Err(ApiError::Unexpected),
            "weaker KDF refused"
        );
        assert_eq!(
            api.kdf("x").map(|_| ()),
            Err(ApiError::Unexpected),
            "bad salt refused"
        );
        assert_eq!(api.kdf("x").map(|_| ()), Err(ApiError::Rejected));
    }

    #[test]
    fn sign_up_sends_the_kdf_and_codes_map_to_errors() {
        let (base, seen) = server(vec![
            (200, vec![], r#"{"token":null,"user":{}}"#),
            (
                400,
                vec![],
                r#"{"message":"Invalid OTP","code":"INVALID_OTP"}"#,
            ),
            (
                400,
                vec![],
                r#"{"message":"OTP expired","code":"OTP_EXPIRED"}"#,
            ),
            (
                403,
                vec![],
                r#"{"message":"Too many attempts","code":"TOO_MANY_ATTEMPTS"}"#,
            ),
            (200, vec![], r#"{"status":true}"#),
            (200, vec![], r#"{"success":true}"#),
        ]);
        let api = HttpAccountClient::new(base);
        api.sign_up("sam@example.com", "sam", HEX, "B465J6Y1apD8t4WQw10Jzg")
            .expect("sign up");
        assert_eq!(
            api.verify_email("sam@example.com", "000000"),
            Err(ApiError::WrongCode)
        );
        assert_eq!(
            api.verify_email("sam@example.com", "000000"),
            Err(ApiError::CodeExpired)
        );
        assert_eq!(
            api.verify_email("sam@example.com", "000000"),
            Err(ApiError::TooManyAttempts)
        );
        api.verify_email("sam@example.com", "123456")
            .expect("verified");
        api.resend_code("sam@example.com").expect("resent");
        let seen = seen.lock().expect("seen");
        let body: serde_json::Value = serde_json::from_str(&seen[0].body).expect("json");
        assert_eq!(body["kdf"]["params"], "argon2id:m=262144,t=3,p=1:v1");
        assert_eq!(body["kdf"]["salt"], "B465J6Y1apD8t4WQw10Jzg");
        assert_eq!(body["password"], HEX);
        let resend: serde_json::Value = serde_json::from_str(&seen[5].body).expect("json");
        assert_eq!(
            resend,
            json!({ "email": "sam@example.com", "type": "email-verification" })
        );
    }

    #[test]
    fn authenticated_calls_send_the_bearer_token() {
        let (base, seen) = server(vec![
            (
                200,
                vec![],
                r#"{"id":"u1","email":"sam@example.com","role":"owner","cloud":{"source":"owner","status":"active","ends_at":null},"hasKeys":true}"#,
            ),
            (404, vec![], r#"{"error":"No keys yet"}"#),
            (
                200,
                vec![],
                r#"{"wrapped_by_password":"aaaa","wrapped_by_recovery":"bbbb","version":1}"#,
            ),
            (201, vec![], r#"{"ok":true}"#),
            (409, vec![], r#"{"error":"Keys already set up"}"#),
            (
                200,
                vec![],
                r#"[{"expiresAt":"2027-01-16T12:53:42.460Z","token":"raw123","createdAt":"2026-11-17T12:53:42.460Z","updatedAt":"2026-11-17T12:54:00.000Z","ipAddress":"127.0.0.1","userAgent":"Ottero/0.1.3 (macOS)","userId":"u1","impersonatedBy":null,"id":"s1"},{"token":"other","id":"s2","userAgent":null,"createdAt":null,"updatedAt":null}]"#,
            ),
            (200, vec![], r#"{"status":true}"#),
            (401, vec![], r#"{"error":"Sign in again"}"#),
            (401, vec![], r#"{"error":"Sign in again"}"#),
        ]);
        let api = HttpAccountClient::new(base);
        let token = SessionToken::new("raw123.sig".into());
        let me = api.me(&token).expect("me");
        assert_eq!(me.plan(), ottero_core::account::Plan::Owner);
        assert!(me.has_keys);
        assert_eq!(api.get_keys(&token), Ok(None));
        assert_eq!(
            api.get_keys(&token).expect("keys").map(|k| k.by_password),
            Some("aaaa".into())
        );
        let keys = WrappedKeys {
            by_password: "p".into(),
            by_recovery: "r".into(),
        };
        api.put_keys(&token, &keys).expect("put");
        assert_eq!(api.put_keys(&token, &keys), Err(ApiError::KeysExist));
        let sessions = api.sessions(&token).expect("sessions");
        assert_eq!(sessions.len(), 2);
        assert!(sessions[0].is_current(&token));
        assert!(!sessions[1].is_current(&token));
        assert_eq!(
            sessions[0].user_agent.as_deref(),
            Some("Ottero/0.1.3 (macOS)")
        );
        assert!(sessions[0].last_active.is_some());
        api.revoke_session(&token, sessions[1].token())
            .expect("revoke");
        assert_eq!(api.me(&token).map(|_| ()), Err(ApiError::SessionEnded));
        api.sign_out(&token).expect("already signed out is fine");
        let seen = seen.lock().expect("seen");
        assert!(
            seen.iter()
                .all(|s| s.authorization.as_deref() == Some("Bearer raw123.sig"))
        );
        assert_eq!(seen[6].path, "/api/auth/revoke-session");
        assert_eq!(seen[6].body, r#"{"token":"other"}"#);
    }

    #[test]
    fn unreachable_server_reads_as_offline() {
        // Bind then drop: nothing listens on that port any more.
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("addr")
            .port();
        let api = HttpAccountClient::new(format!("http://127.0.0.1:{port}"));
        assert_eq!(api.kdf("a@b.co").map(|_| ()), Err(ApiError::Offline));
        assert!(ApiError::Offline.message().contains("connection"));
    }

    #[test]
    fn redirects_are_not_followed() {
        let (base, seen) = server(vec![(302, vec![("location", "http://evil.example/")], "")]);
        let api = HttpAccountClient::new(base);
        assert!(api.sign_in("a@b.co", HEX).is_err());
        assert_eq!(seen.lock().expect("seen").len(), 1);
    }

    const URI: &str = "otpauth://totp/Ottero:sam%40example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Ottero&digits=6&period=30";

    #[test]
    fn two_step_sign_in_sends_the_challenge_cookie_and_ignores_the_first_token() {
        let (base, seen) = server(vec![
            // Captured from the real API: the first step carries an emptied
            // session cookie, the signed two-step cookie, and no session.
            (
                200,
                vec![
                    ("set-auth-token", "not-a-session"),
                    (
                        "set-cookie",
                        "better-auth.session_token=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax",
                    ),
                    (
                        "set-cookie",
                        "better-auth.two_factor=2fa-abc.sig%3D; Max-Age=600; Path=/; HttpOnly; SameSite=Lax",
                    ),
                ],
                r#"{"twoFactorRedirect":true,"twoFactorMethods":["totp"]}"#,
            ),
            (
                401,
                vec![],
                r#"{"message":"Invalid code","code":"INVALID_CODE"}"#,
            ),
            (
                200,
                vec![("set-auth-token", "sess.sig")],
                r#"{"token":"sess"}"#,
            ),
            (
                401,
                vec![],
                r#"{"message":"Invalid backup code","code":"INVALID_BACKUP_CODE"}"#,
            ),
            (
                401,
                vec![],
                r#"{"message":"Invalid two factor cookie","code":"INVALID_TWO_FACTOR_COOKIE"}"#,
            ),
            (
                400,
                vec![],
                r#"{"message":"Too many attempts. Please request a new code.","code":"TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE"}"#,
            ),
            (
                429,
                vec![],
                r#"{"message":"Too many failed verification attempts.","code":"ACCOUNT_TEMPORARILY_LOCKED"}"#,
            ),
            (
                429,
                vec![],
                r#"{"message":"Too many requests. Please try again later."}"#,
            ),
            (
                200,
                vec![(
                    "set-cookie",
                    "__Secure-better-auth.two_factor=2fa-x.sig; Path=/; Secure",
                )],
                r#"{"twoFactorRedirect":true}"#,
            ),
            (200, vec![], r#"{"twoFactorRedirect":true}"#),
        ]);
        let api = HttpAccountClient::new(base.clone());
        let SignInOutcome::TwoStep(challenge) = api.sign_in("sam@example.com", HEX).expect("ok")
        else {
            panic!("needs the second step");
        };
        assert_eq!(challenge.expose(), "better-auth.two_factor=2fa-abc.sig%3D");
        assert_eq!(format!("{challenge:?}"), "TwoStepChallenge([redacted])");
        let verify = |factor, code| {
            api.verify_second_factor(&challenge, factor, code)
                .map(|_| ())
        };
        assert_eq!(
            verify(SecondFactor::Authenticator, "000000"),
            Err(ApiError::WrongTwoStepCode)
        );
        let token = api
            .verify_second_factor(&challenge, SecondFactor::Authenticator, "287082")
            .expect("session");
        assert_eq!(token.expose(), "sess.sig");
        assert_eq!(
            verify(SecondFactor::BackupCode, "aaaaa-bbbbb"),
            Err(ApiError::WrongBackupCode)
        );
        assert_eq!(
            verify(SecondFactor::Authenticator, "1"),
            Err(ApiError::TwoStepExpired)
        );
        assert_eq!(
            verify(SecondFactor::Authenticator, "1"),
            Err(ApiError::TwoStepTooManyCodes)
        );
        assert_eq!(
            verify(SecondFactor::Authenticator, "1"),
            Err(ApiError::TwoStepLocked)
        );
        assert_eq!(
            verify(SecondFactor::Authenticator, "1"),
            Err(ApiError::TooManyAttempts),
            "the rate limiter is not the lockout"
        );
        let SignInOutcome::TwoStep(secure) = api.sign_in("a@b.co", HEX).expect("ok") else {
            panic!("https cookie name");
        };
        assert_eq!(secure.expose(), "__Secure-better-auth.two_factor=2fa-x.sig");
        assert_eq!(
            api.sign_in("a@b.co", HEX).map(|_| ()),
            Err(ApiError::Unexpected),
            "a redirect without its cookie can't continue"
        );
        let seen = seen.lock().expect("seen");
        assert_eq!(seen[1].path, "/api/auth/two-factor/verify-totp");
        assert_eq!(
            seen[1].cookie.as_deref(),
            Some("better-auth.two_factor=2fa-abc.sig%3D")
        );
        assert!(
            seen[1].authorization.is_none(),
            "no bearer on the second step"
        );
        assert_eq!(
            seen[1].origin.as_deref(),
            Some(base.as_str()),
            "the server's own origin passes its CSRF check"
        );
        assert!(
            seen[0].origin.is_none(),
            "only cookie requests name an origin"
        );
        assert_eq!(seen[2].body, r#"{"code":"287082"}"#);
        assert_eq!(seen[3].path, "/api/auth/two-factor/verify-backup-code");
    }

    #[test]
    fn turning_two_step_on_and_off_uses_the_login_key_and_keeps_the_new_token() {
        let enabled = format!(
            r#"{{"method":"totp","totpURI":"{URI}","backupCodes":["aaaaa-11111","bbbbb-22222"]}}"#
        );
        let enabled: &'static str = Box::leak(enabled.into_boxed_str());
        let (base, seen) = server(vec![
            (200, vec![], enabled),
            (
                401,
                vec![],
                r#"{"message":"Invalid code","code":"INVALID_CODE"}"#,
            ),
            (
                200,
                vec![("set-auth-token", "new.sig")],
                r#"{"token":"new"}"#,
            ),
            (
                400,
                vec![],
                r#"{"message":"TOTP is already enabled","code":"TOTP_ALREADY_ENABLED"}"#,
            ),
            (
                200,
                vec![],
                r#"{"status":true,"backupCodes":["ccccc-33333"]}"#,
            ),
            (
                200,
                vec![("set-auth-token", "after-off.sig")],
                r#"{"status":true}"#,
            ),
            (
                400,
                vec![],
                r#"{"message":"Invalid password","code":"INVALID_PASSWORD"}"#,
            ),
            (
                401,
                vec![],
                r#"{"message":"Unauthorized","code":"UNAUTHORIZED"}"#,
            ),
            (
                200,
                vec![],
                r#"{"totpURI":"https://evil.example/","backupCodes":["a"]}"#,
            ),
            (200, vec![], r#"{"status":true}"#),
        ]);
        let api = HttpAccountClient::new(base);
        let token = SessionToken::new("old.sig".into());
        let setup = api.enable_two_factor(&token, HEX).expect("setup");
        assert_eq!(setup.uri(), URI);
        assert_eq!(setup.setup_key(), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(setup.backup_codes().len(), 2);
        assert_eq!(
            api.confirm_two_factor(&token, "000000").map(|_| ()),
            Err(ApiError::WrongTwoStepCode),
            "a wrong code is not a lost session"
        );
        let new = api
            .confirm_two_factor(&token, "287082")
            .expect("on")
            .expect("replacement token");
        assert_eq!(new.expose(), "new.sig");
        assert_eq!(
            api.enable_two_factor(&new, HEX).map(|_| ()),
            Err(ApiError::TwoStepChanged)
        );
        let codes = api.regenerate_backup_codes(&new, HEX).expect("codes");
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].as_str(), "ccccc-33333");
        let after = api.disable_two_factor(&new, HEX).expect("off");
        assert_eq!(
            after.map(|t| t.expose().to_owned()).as_deref(),
            Some("after-off.sig")
        );
        assert_eq!(
            api.disable_two_factor(&new, HEX).map(|_| ()),
            Err(ApiError::WrongCredentials)
        );
        assert_eq!(
            api.disable_two_factor(&new, HEX).map(|_| ()),
            Err(ApiError::SessionEnded)
        );
        assert_eq!(
            api.enable_two_factor(&new, HEX).map(|_| ()),
            Err(ApiError::Unexpected),
            "only an otpauth TOTP link is shown as a QR code"
        );
        assert_eq!(
            api.regenerate_backup_codes(&new, HEX).map(|_| ()),
            Err(ApiError::Unexpected),
            "no codes in the answer"
        );
        let seen = seen.lock().expect("seen");
        assert_eq!(seen[0].path, "/api/auth/two-factor/enable");
        assert_eq!(seen[0].body, format!(r#"{{"password":"{HEX}"}}"#));
        assert_eq!(seen[0].authorization.as_deref(), Some("Bearer old.sig"));
        assert_eq!(seen[3].authorization.as_deref(), Some("Bearer new.sig"));
        assert_eq!(seen[4].path, "/api/auth/two-factor/generate-backup-codes");
        assert_eq!(seen[5].path, "/api/auth/two-factor/disable");
    }

    #[test]
    fn setup_keys_come_only_from_totp_links() {
        assert_eq!(
            setup_key_of(URI).as_deref().map(String::as_str),
            Some("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")
        );
        assert!(setup_key_of("otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ").is_none());
        assert!(setup_key_of("otpauth://totp/x?secret=short").is_none());
        assert!(setup_key_of("otpauth://totp/x?secret=GEZDGNBVGY3TQOJ1").is_none());
        assert!(setup_key_of("otpauth://totp/x").is_none());
        assert!(TwoStepSetup::new(URI.into(), vec![]).is_err());
    }
}
