//! Ottero Cloud account rules: email and password checks, the plan shown
//! in Settings → Account, and the account state machine.
//!
//! ```text
//! SignedOut ──StartSignUp──▶ SigningUp ──CodeSent──▶ AwaitingCode ──Verified──▶ EmergencyKit
//! EmergencyKit ──KitSaved──▶ SignedIn
//! SignedOut ──StartSignIn──▶ SigningIn ──SignedIn──▶ SignedIn
//!                                     ├──SecretKeyMissing──▶ NeedsSecretKey ──StartSignIn──▶ SigningIn
//!                                     ├──NotVerified──▶ AwaitingCode     (sign-up never confirmed)
//!                                     ├──KeysMissing──▶ EmergencyKit     (verified, keys never saved)
//!                                     └──TwoStepRequired──▶ TwoStep      (two-step verification is on)
//! AwaitingCode ──SignedIn──▶ SignedIn         (code confirmed, keys already saved)
//! AwaitingCode ──KeysMissing──▶ EmergencyKit
//! AwaitingCode ──TwoStepRequired──▶ TwoStep
//! TwoStep ──SignedIn──▶ SignedIn              (authenticator or backup code accepted)
//! TwoStep ──KeysMissing──▶ EmergencyKit
//! SigningUp | SigningIn | NeedsSecretKey | TwoStep ──Failed──▶ Error
//! Error ──StartSignUp / StartSignIn──▶ SigningUp / SigningIn   (try again)
//! SignedIn ──PlanChanged──▶ SignedIn
//! any but SignedOut ──Cancel / SignOut──▶ SignedOut
//! SignedIn ──SessionEnded──▶ SignedOut         (revoked elsewhere, blocked, expired)
//! ```
//!
//! `TwoStep` is the second step of signing in when the account has two-step
//! verification on: the password (and Secret Key) were right, and the server
//! now wants the 6-digit code from an authenticator app or a backup code.
//! It fails into `Error { flow: SignIn }` only when that sign-in attempt is
//! over (it expired, or too many wrong codes): the password is needed again.
//!
//! `Error` remembers which form failed (sign-up or sign-in) so the screen
//! can show it again with the message next to the field. A wrong code or a
//! failed key upload keeps its screen (`AwaitingCode`, `TwoStep`,
//! `EmergencyKit`); the app shows that error in place instead of changing state. Every other pair is refused
//! (`next` returns `None`), so a late network result can't move a screen
//! the user already left. Network, crypto and storage live in the app; the
//! state machine only names where the user is.

use crate::vault::health::{Strength, estimate};
use jiff::Timestamp;

/// Passwords shorter than this are refused (plan 06 §2).
pub const MIN_PASSWORD_CHARS: usize = 10;
/// Emails longer than this are refused (RFC 5321 path limit, as the server).
pub const MAX_EMAIL_CHARS: usize = 254;

/// `trim` + lowercase, matching the server and the key derivation.
#[must_use]
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailProblem {
    Empty,
    Invalid,
    TooLong,
}

impl EmailProblem {
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "Enter your email.",
            Self::Invalid => "Enter an email like name@example.com.",
            Self::TooLong => "That email is too long.",
        }
    }
}

/// A normalized email, or what is wrong with it. Deliberately simple: one
/// `@`, a local part, a dotted domain, no spaces. The code email proves the rest.
pub fn validate_email(input: &str) -> Result<String, EmailProblem> {
    let email = normalize_email(input);
    if email.is_empty() {
        return Err(EmailProblem::Empty);
    }
    if email.chars().count() > MAX_EMAIL_CHARS {
        return Err(EmailProblem::TooLong);
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err(EmailProblem::Invalid);
    };
    let labels_ok = domain.split('.').count() >= 2
        && domain
            .split('.')
            .all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'));
    if local.is_empty()
        || domain.contains('@')
        || !labels_ok
        || email.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(EmailProblem::Invalid);
    }
    Ok(email)
}

/// Often-breached passwords that pass the length rule. zxcvbn knows far
/// more; this list exists so the most common ones get a plain warning.
const COMMON: &[&str] = &[
    "1234567890",
    "12345678910",
    "0123456789",
    "0987654321",
    "1111111111",
    "1q2w3e4r5t",
    "1qaz2wsx3edc",
    "qwertyuiop",
    "asdfghjkl;",
    "zxcvbnm123",
    "qwerty1234",
    "qwerty12345",
    "qwertyuiop1",
    "password12",
    "password123",
    "password1234",
    "password!1",
    "passw0rd123",
    "p@ssw0rd123",
    "iloveyou12",
    "iloveyou123",
    "abcdefghij",
    "abcd123456",
    "abc1234567",
    "a1b2c3d4e5",
    "letmein123",
    "welcome123",
    "welcome1234",
    "football123",
    "baseball123",
    "princess123",
    "sunshine123",
    "monkey12345",
    "dragon12345",
    "superman123",
    "starwars123",
    "trustno1234",
    "michael123",
    "computer123",
    "internet123",
    "changeme123",
    "admin12345",
    "administrator",
    "qazwsxedcrfv",
    "1234qwerty",
    "123456789a",
    "123456789q",
    "1234567890a",
    "987654321a",
    "11223344556",
    "1122334455",
    "123123123123",
    "1231231234",
    "0000000000",
    "9999999999",
    "passwordpassword",
    "loveyou1234",
    "whatever123",
    "secret1234",
    "master12345",
];

/// Strength of a new account password, and what (if anything) blocks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordReport {
    pub strength: Strength,
    /// A short, plain reason the password can't be used yet.
    pub problem: Option<&'static str>,
    /// One of the most common passwords (also forces `problem`).
    pub common: bool,
    /// zxcvbn's hint, when it has one.
    pub hint: Option<String>,
}

/// Checks a new account password. `email` is context (a password made of
/// the email is weak).
#[must_use]
pub fn check_password(password: &str, email: &str) -> PasswordReport {
    let email = normalize_email(email);
    let local = email.split('@').next().unwrap_or_default().to_owned();
    let e = estimate(password, &[email.as_str(), local.as_str(), "ottero"]);
    let lowered = password.to_lowercase();
    let common = COMMON.contains(&lowered.as_str());
    let problem = if password.chars().count() < MIN_PASSWORD_CHARS {
        Some("Use at least 10 characters.")
    } else if common {
        Some("That's one of the most common passwords. Choose another.")
    } else if !email.is_empty() && lowered.contains(&email) {
        Some("Don't use your email in your password.")
    } else if e.strength < Strength::Fair {
        Some("Choose something harder to guess.")
    } else {
        None
    };
    PasswordReport {
        strength: if common {
            Strength::VeryWeak
        } else {
            e.strength
        },
        problem,
        common,
        hint: e.hint,
    }
}

/// Whether this device holds the Secret Key for an email. A new device
/// must ask for it (Emergency Kit); afterwards it's never asked again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKeyPresence {
    Stored,
    Missing,
}

/// Where Ottero Cloud access comes from (server `entitlement.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudSource {
    Paid,
    Granted,
    Owner,
}

impl CloudSource {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dodo" | "razorpay" => Some(Self::Paid),
            "grant" => Some(Self::Granted),
            "owner" => Some(Self::Owner),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paid => "razorpay",
            Self::Granted => "grant",
            Self::Owner => "owner",
        }
    }
}

/// The account's plan as Settings shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Free,
    Cloud {
        source: CloudSource,
        ends_at: Option<Timestamp>,
    },
    /// The owner account: Cloud forever.
    Owner,
}

impl Plan {
    /// From the server's `role` and `cloud` (`None` when it is `null`).
    /// An unknown source still means Cloud is active; it reads as granted.
    #[must_use]
    pub fn from_parts(role: &str, cloud: Option<(&str, Option<Timestamp>)>) -> Self {
        if role == "owner" {
            return Self::Owner;
        }
        match cloud {
            None => Self::Free,
            Some((source, ends_at)) => match CloudSource::parse(source) {
                Some(CloudSource::Owner) => Self::Owner,
                Some(source) => Self::Cloud { source, ends_at },
                None => Self::Cloud {
                    source: CloudSource::Granted,
                    ends_at,
                },
            },
        }
    }

    #[must_use]
    pub fn has_cloud(self) -> bool {
        !matches!(self, Self::Free)
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Cloud { .. } => "Ottero Cloud",
            Self::Owner => "Owner",
        }
    }

    /// One line under the plan name. `today_label` formats `ends_at`.
    #[must_use]
    pub fn detail(self, now: Timestamp, format_date: impl Fn(Timestamp) -> String) -> String {
        match self {
            Self::Free => "Every feature on this computer. Sync needs Ottero Cloud.".into(),
            Self::Owner => "Ottero Cloud, always included.".into(),
            Self::Cloud { source, ends_at } => {
                let from = match source {
                    CloudSource::Paid => "Subscription",
                    CloudSource::Granted | CloudSource::Owner => "Granted by Ottero",
                };
                match ends_at {
                    Some(end) if end > now => format!("{from}, until {}.", format_date(end)),
                    Some(_) => format!("{from}, ended."),
                    None => format!("{from}."),
                }
            }
        }
    }
}

/// The kind of device a session's user agent names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Computer,
    Phone,
    Browser,
    Unknown,
}

/// A readable name for a session: `Ottero/0.1.3 (macOS)` → "Ottero on macOS".
#[must_use]
pub fn device_label(user_agent: Option<&str>) -> (String, DeviceKind) {
    let Some(agent) = user_agent.map(str::trim).filter(|a| !a.is_empty()) else {
        return ("Unknown device".into(), DeviceKind::Unknown);
    };
    if let Some(rest) = agent.strip_prefix("Ottero/") {
        let os = rest
            .split_once('(')
            .and_then(|(_, os)| os.split(')').next())
            .map(str::trim)
            .filter(|os| !os.is_empty());
        let kind = match os {
            Some("iOS" | "Android") => DeviceKind::Phone,
            _ => DeviceKind::Computer,
        };
        return (
            os.map_or_else(|| "Ottero".to_owned(), |os| format!("Ottero on {os}")),
            kind,
        );
    }
    let os = [
        ("iPhone", "iPhone"),
        ("Android", "Android"),
        ("Mac OS X", "Mac"),
        ("Macintosh", "Mac"),
        ("Windows", "Windows"),
        ("Linux", "Linux"),
    ]
    .into_iter()
    .find(|(needle, _)| agent.contains(needle))
    .map(|(_, name)| name);
    match os {
        Some(os) => (format!("Web browser on {os}"), DeviceKind::Browser),
        None => ("Web browser".into(), DeviceKind::Browser),
    }
}

/// "Active now", "Active 5 min ago", "Active 3 h ago", "Active 2 days ago".
#[must_use]
pub fn active_label(at: Timestamp, now: Timestamp) -> String {
    let secs = now.as_second().saturating_sub(at.as_second()).max(0);
    match secs {
        0..120 => "Active now".into(),
        120..3_600 => format!("Active {} min ago", secs / 60),
        3_600..86_400 => format!("Active {} h ago", secs / 3_600),
        86_400..172_800 => "Active yesterday".into(),
        _ => format!("Active {} days ago", secs / 86_400),
    }
}

/// Digits in a typed or pasted authenticator code, at most six
/// ("123 456", "123-456" and "123456" are the same code).
#[must_use]
pub fn code_digits(input: &str) -> String {
    input.chars().filter(char::is_ascii_digit).take(6).collect()
}

/// "123 456" while typing: digits only, a space after the third.
#[must_use]
pub fn group_code(digits: &str) -> String {
    if digits.len() > 3 {
        format!("{} {}", &digits[..3], &digits[3..])
    } else {
        digits.to_owned()
    }
}

/// A typed backup code in the server's form (`ab12C-De34f`), or `None` if
/// it can't be one. Spaces are ignored and a missing dash is added; case is
/// kept because the codes are case-sensitive.
#[must_use]
pub fn normalize_backup_code(input: &str) -> Option<String> {
    let compact: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    (compact.len() == 10 && compact.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| format!("{}-{}", &compact[..5], &compact[5..]))
}

/// A base32 setup key in groups of four for reading aloud or typing into an
/// authenticator app: "JBSW Y3DP EHPK 3PXP".
#[must_use]
pub fn group_setup_key(secret: &str) -> String {
    secret
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Which form a failure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    SignUp,
    SignIn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountState {
    SignedOut,
    /// Deriving keys and creating the account.
    SigningUp {
        email: String,
    },
    /// Waiting for the 6-digit code from the email.
    AwaitingCode {
        email: String,
    },
    /// Showing the Secret Key and recovery code until the user saved them.
    EmergencyKit {
        email: String,
    },
    SigningIn {
        email: String,
    },
    /// This device has no Secret Key for `email`; ask for it.
    NeedsSecretKey {
        email: String,
    },
    /// Two-step verification is on: waiting for the authenticator (or
    /// backup) code that finishes signing in.
    TwoStep {
        email: String,
    },
    SignedIn {
        email: String,
        plan: Plan,
    },
    Error {
        email: String,
        flow: Flow,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountEvent {
    StartSignUp { email: String },
    CodeSent,
    Verified,
    KitSaved { plan: Plan },
    StartSignIn { email: String },
    SecretKeyMissing,
    NotVerified,
    KeysMissing,
    TwoStepRequired,
    SignedIn { plan: Plan },
    PlanChanged { plan: Plan },
    Failed { message: String },
    Cancel,
    SignOut,
    SessionEnded,
}

impl AccountState {
    #[must_use]
    pub fn email(&self) -> Option<&str> {
        match self {
            Self::SignedOut => None,
            Self::SigningUp { email }
            | Self::AwaitingCode { email }
            | Self::EmergencyKit { email }
            | Self::SigningIn { email }
            | Self::NeedsSecretKey { email }
            | Self::TwoStep { email }
            | Self::SignedIn { email, .. }
            | Self::Error { email, .. } => Some(email),
        }
    }

    /// Waiting on the network or Argon2.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::SigningUp { .. } | Self::SigningIn { .. })
    }

    #[must_use]
    pub fn is_signed_in(&self) -> bool {
        matches!(self, Self::SignedIn { .. })
    }

    fn flow(&self) -> Option<Flow> {
        match self {
            Self::SigningUp { .. } => Some(Flow::SignUp),
            Self::SigningIn { .. } | Self::NeedsSecretKey { .. } | Self::TwoStep { .. } => {
                Some(Flow::SignIn)
            }
            _ => None,
        }
    }
}

/// The next state, or `None` if `event` isn't allowed in `state`.
#[must_use]
pub fn next(state: &AccountState, event: AccountEvent) -> Option<AccountState> {
    use AccountEvent as E;
    use AccountState as S;
    let email = || state.email().unwrap_or_default().to_owned();
    match (state, event) {
        (S::SignedOut | S::Error { .. }, E::StartSignUp { email }) => Some(S::SigningUp { email }),
        (S::SignedOut | S::Error { .. } | S::NeedsSecretKey { .. }, E::StartSignIn { email }) => {
            Some(S::SigningIn { email })
        }
        (S::SigningUp { .. }, E::CodeSent) => Some(S::AwaitingCode { email: email() }),
        (S::AwaitingCode { .. }, E::Verified) => Some(S::EmergencyKit { email: email() }),
        (S::EmergencyKit { .. }, E::KitSaved { plan }) => Some(S::SignedIn {
            email: email(),
            plan,
        }),
        (S::SigningIn { .. }, E::SecretKeyMissing) => Some(S::NeedsSecretKey { email: email() }),
        (S::SigningIn { .. }, E::NotVerified) => Some(S::AwaitingCode { email: email() }),
        (S::SigningIn { .. } | S::AwaitingCode { .. }, E::TwoStepRequired) => {
            Some(S::TwoStep { email: email() })
        }
        (S::SigningIn { .. } | S::AwaitingCode { .. } | S::TwoStep { .. }, E::KeysMissing) => {
            Some(S::EmergencyKit { email: email() })
        }
        (
            S::SigningIn { .. } | S::AwaitingCode { .. } | S::TwoStep { .. },
            E::SignedIn { plan },
        ) => Some(S::SignedIn {
            email: email(),
            plan,
        }),
        (S::SignedIn { .. }, E::PlanChanged { plan }) => Some(S::SignedIn {
            email: email(),
            plan,
        }),
        (
            s @ (S::SigningUp { .. }
            | S::SigningIn { .. }
            | S::NeedsSecretKey { .. }
            | S::TwoStep { .. }),
            E::Failed { message },
        ) => Some(S::Error {
            email: email(),
            flow: s.flow().unwrap_or(Flow::SignIn),
            message,
        }),
        (S::SignedOut, E::Cancel | E::SignOut) => None,
        (_, E::Cancel | E::SignOut) => Some(S::SignedOut),
        (S::SignedIn { .. }, E::SessionEnded) => Some(S::SignedOut),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_states() -> Vec<AccountState> {
        let e = || "sam@example.com".to_owned();
        vec![
            AccountState::SignedOut,
            AccountState::SigningUp { email: e() },
            AccountState::AwaitingCode { email: e() },
            AccountState::EmergencyKit { email: e() },
            AccountState::SigningIn { email: e() },
            AccountState::NeedsSecretKey { email: e() },
            AccountState::TwoStep { email: e() },
            AccountState::SignedIn {
                email: e(),
                plan: Plan::Free,
            },
            AccountState::Error {
                email: e(),
                flow: Flow::SignIn,
                message: "x".into(),
            },
        ]
    }

    fn all_events() -> Vec<AccountEvent> {
        let e = || "sam@example.com".to_owned();
        vec![
            AccountEvent::StartSignUp { email: e() },
            AccountEvent::CodeSent,
            AccountEvent::Verified,
            AccountEvent::KitSaved { plan: Plan::Free },
            AccountEvent::StartSignIn { email: e() },
            AccountEvent::SecretKeyMissing,
            AccountEvent::NotVerified,
            AccountEvent::KeysMissing,
            AccountEvent::TwoStepRequired,
            AccountEvent::SignedIn { plan: Plan::Free },
            AccountEvent::PlanChanged { plan: Plan::Owner },
            AccountEvent::Failed {
                message: "Offline".into(),
            },
            AccountEvent::Cancel,
            AccountEvent::SignOut,
            AccountEvent::SessionEnded,
        ]
    }

    fn name(s: &AccountState) -> &'static str {
        match s {
            AccountState::SignedOut => "SignedOut",
            AccountState::SigningUp { .. } => "SigningUp",
            AccountState::AwaitingCode { .. } => "AwaitingCode",
            AccountState::EmergencyKit { .. } => "EmergencyKit",
            AccountState::SigningIn { .. } => "SigningIn",
            AccountState::NeedsSecretKey { .. } => "NeedsSecretKey",
            AccountState::TwoStep { .. } => "TwoStep",
            AccountState::SignedIn { .. } => "SignedIn",
            AccountState::Error { .. } => "Error",
        }
    }

    fn event_name(e: &AccountEvent) -> &'static str {
        match e {
            AccountEvent::StartSignUp { .. } => "StartSignUp",
            AccountEvent::CodeSent => "CodeSent",
            AccountEvent::Verified => "Verified",
            AccountEvent::KitSaved { .. } => "KitSaved",
            AccountEvent::StartSignIn { .. } => "StartSignIn",
            AccountEvent::SecretKeyMissing => "SecretKeyMissing",
            AccountEvent::NotVerified => "NotVerified",
            AccountEvent::KeysMissing => "KeysMissing",
            AccountEvent::TwoStepRequired => "TwoStepRequired",
            AccountEvent::SignedIn { .. } => "SignedIn",
            AccountEvent::PlanChanged { .. } => "PlanChanged",
            AccountEvent::Failed { .. } => "Failed",
            AccountEvent::Cancel => "Cancel",
            AccountEvent::SignOut => "SignOut",
            AccountEvent::SessionEnded => "SessionEnded",
        }
    }

    /// The whole table: every (state, event) pair and where it leads.
    #[test]
    fn transition_table_is_exact() {
        let busy_or_flow = ["SigningUp", "SigningIn", "NeedsSecretKey", "TwoStep"];
        let allowed: &[(&str, &str, &str)] = &[
            ("SignedOut", "StartSignUp", "SigningUp"),
            ("Error", "StartSignUp", "SigningUp"),
            ("SignedOut", "StartSignIn", "SigningIn"),
            ("Error", "StartSignIn", "SigningIn"),
            ("NeedsSecretKey", "StartSignIn", "SigningIn"),
            ("SigningUp", "CodeSent", "AwaitingCode"),
            ("AwaitingCode", "Verified", "EmergencyKit"),
            ("EmergencyKit", "KitSaved", "SignedIn"),
            ("SigningIn", "SecretKeyMissing", "NeedsSecretKey"),
            ("SigningIn", "NotVerified", "AwaitingCode"),
            ("SigningIn", "KeysMissing", "EmergencyKit"),
            ("AwaitingCode", "KeysMissing", "EmergencyKit"),
            ("TwoStep", "KeysMissing", "EmergencyKit"),
            ("SigningIn", "TwoStepRequired", "TwoStep"),
            ("AwaitingCode", "TwoStepRequired", "TwoStep"),
            ("SigningIn", "SignedIn", "SignedIn"),
            ("AwaitingCode", "SignedIn", "SignedIn"),
            ("TwoStep", "SignedIn", "SignedIn"),
            ("SignedIn", "PlanChanged", "SignedIn"),
            ("SignedIn", "SessionEnded", "SignedOut"),
        ];
        for state in all_states() {
            for event in all_events() {
                let (s, e) = (name(&state), event_name(&event));
                let expected = allowed
                    .iter()
                    .find(|(from, ev, _)| *from == s && *ev == e)
                    .map(|(_, _, to)| *to)
                    .or_else(|| (e == "Failed" && busy_or_flow.contains(&s)).then_some("Error"))
                    .or_else(|| {
                        (matches!(e, "Cancel" | "SignOut") && s != "SignedOut")
                            .then_some("SignedOut")
                    });
                let got = next(&state, event.clone());
                assert_eq!(got.as_ref().map(name), expected, "{s} + {e}");
            }
        }
    }

    #[test]
    fn transitions_keep_the_email_and_error_remembers_the_flow() {
        let s = next(
            &AccountState::SignedOut,
            AccountEvent::StartSignUp {
                email: "a@b.co".into(),
            },
        )
        .expect("start");
        let err = next(
            &s,
            AccountEvent::Failed {
                message: "Offline".into(),
            },
        )
        .expect("failed");
        assert_eq!(
            err,
            AccountState::Error {
                email: "a@b.co".into(),
                flow: Flow::SignUp,
                message: "Offline".into()
            }
        );
        let s = next(&s, AccountEvent::CodeSent).expect("sent");
        assert_eq!(s.email(), Some("a@b.co"));
        assert_eq!(
            next(
                &s,
                AccountEvent::Failed {
                    message: "x".into()
                }
            ),
            None
        );
        let s = next(
            &AccountState::AwaitingCode {
                email: "a@b.co".into(),
            },
            AccountEvent::Verified,
        )
        .expect("verified");
        let s = next(&s, AccountEvent::KitSaved { plan: Plan::Owner }).expect("saved");
        assert_eq!(
            s,
            AccountState::SignedIn {
                email: "a@b.co".into(),
                plan: Plan::Owner
            }
        );
        assert!(s.is_signed_in() && !s.is_busy());
    }

    #[test]
    fn two_step_sign_in_keeps_the_email_and_expiry_returns_to_the_password() {
        let s = next(
            &AccountState::SignedOut,
            AccountEvent::StartSignIn {
                email: "a@b.co".into(),
            },
        )
        .expect("start");
        let two = next(&s, AccountEvent::TwoStepRequired).expect("two-step");
        assert_eq!(
            two,
            AccountState::TwoStep {
                email: "a@b.co".into()
            }
        );
        assert!(!two.is_busy() && !two.is_signed_in());
        let expired = next(
            &two,
            AccountEvent::Failed {
                message: "Expired".into(),
            },
        )
        .expect("expired");
        assert_eq!(
            expired,
            AccountState::Error {
                email: "a@b.co".into(),
                flow: Flow::SignIn,
                message: "Expired".into()
            },
            "back to the sign-in form"
        );
        let done = next(&two, AccountEvent::SignedIn { plan: Plan::Free }).expect("in");
        assert!(done.is_signed_in());
        assert_eq!(done.email(), Some("a@b.co"));
        assert_eq!(
            next(&two, AccountEvent::Cancel),
            Some(AccountState::SignedOut)
        );
        assert_eq!(next(&two, AccountEvent::SessionEnded), None);
    }

    #[test]
    fn codes_and_setup_keys_are_normalized() {
        assert_eq!(code_digits(" 123 456 "), "123456");
        assert_eq!(code_digits("123-4567"), "123456");
        assert_eq!(code_digits("abc"), "");
        assert_eq!(group_code("1234"), "123 4");
        assert_eq!(group_code("12"), "12");
        assert_eq!(
            normalize_backup_code(" Ab12c De34F "),
            Some("Ab12c-De34F".into())
        );
        assert_eq!(
            normalize_backup_code("Ab12c-De34F"),
            Some("Ab12c-De34F".into())
        );
        assert_eq!(normalize_backup_code("Ab12c-De34"), None);
        assert_eq!(normalize_backup_code("Ab12c-De3!F"), None);
        assert_eq!(normalize_backup_code(""), None);
        assert_eq!(
            group_setup_key("JBSWY3DPEHPK3PXPAB"),
            "JBSW Y3DP EHPK 3PXP AB"
        );
    }

    #[test]
    fn emails_are_validated_and_normalized() {
        assert_eq!(
            validate_email("  Sam@Example.COM "),
            Ok("sam@example.com".into())
        );
        assert_eq!(validate_email(""), Err(EmailProblem::Empty));
        assert_eq!(validate_email("   "), Err(EmailProblem::Empty));
        for bad in [
            "sam",
            "sam@",
            "@example.com",
            "sam@example",
            "sam@@example.com",
            "s am@example.com",
            "sam@example..com",
            "sam@-x.com",
        ] {
            assert_eq!(validate_email(bad), Err(EmailProblem::Invalid), "{bad}");
        }
        let long = format!("{}@example.com", "a".repeat(250));
        assert_eq!(validate_email(&long), Err(EmailProblem::TooLong));
    }

    #[test]
    fn password_policy() {
        let short = check_password("abc", "sam@example.com");
        assert_eq!(short.problem, Some("Use at least 10 characters."));
        let common = check_password("Password123", "sam@example.com");
        assert!(common.common);
        assert_eq!(common.strength, Strength::VeryWeak);
        assert!(common.problem.is_some_and(|p| p.contains("most common")));
        let email = check_password("sam@example.com!!", "sam@example.com");
        assert!(email.problem.is_some());
        let weak = check_password("aaaaaaaaaaaa", "sam@example.com");
        assert_eq!(weak.problem, Some("Choose something harder to guess."));
        let good = check_password("Tangy-Velvet-Orbit-42!", "sam@example.com");
        assert_eq!(good.problem, None);
        assert!(good.strength >= Strength::Strong);
        assert!(
            COMMON
                .iter()
                .all(|p| p.chars().count() >= MIN_PASSWORD_CHARS)
        );
    }

    #[test]
    fn device_and_activity_labels() {
        assert_eq!(
            device_label(Some("Ottero/0.1.3 (macOS)")),
            ("Ottero on macOS".into(), DeviceKind::Computer)
        );
        assert_eq!(device_label(Some("Ottero/1.0 (iOS)")).1, DeviceKind::Phone);
        assert_eq!(device_label(Some("Ottero/1.0")).0, "Ottero");
        assert_eq!(
            device_label(Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) Safari")),
            ("Web browser on Mac".into(), DeviceKind::Browser)
        );
        assert_eq!(device_label(None).1, DeviceKind::Unknown);
        let now: Timestamp = "2026-11-17T12:00:00Z".parse().expect("ts");
        let ago = |s: i64| Timestamp::from_second(now.as_second() - s).expect("ts");
        assert_eq!(active_label(ago(30), now), "Active now");
        assert_eq!(active_label(ago(600), now), "Active 10 min ago");
        assert_eq!(active_label(ago(7_200), now), "Active 2 h ago");
        assert_eq!(active_label(ago(90_000), now), "Active yesterday");
        assert_eq!(active_label(ago(5 * 86_400), now), "Active 5 days ago");
        assert_eq!(active_label(ago(-60), now), "Active now", "clock skew");
    }

    #[test]
    fn plans_from_server_fields() {
        let t: Timestamp = "2030-01-01T00:00:00Z".parse().expect("ts");
        let now: Timestamp = "2029-01-01T00:00:00Z".parse().expect("ts");
        assert_eq!(Plan::from_parts("user", None), Plan::Free);
        assert_eq!(Plan::from_parts("owner", None), Plan::Owner);
        assert_eq!(Plan::from_parts("user", Some(("owner", None))), Plan::Owner);
        let granted = Plan::from_parts("user", Some(("grant", Some(t))));
        assert_eq!(
            granted,
            Plan::Cloud {
                source: CloudSource::Granted,
                ends_at: Some(t)
            }
        );
        assert_eq!(granted.label(), "Ottero Cloud");
        assert_eq!(
            granted.detail(now, |_| "Jan 1, 2030".into()),
            "Granted by Ottero, until Jan 1, 2030."
        );
        assert_eq!(
            granted.detail(t, |_| String::new()),
            "Granted by Ottero, ended."
        );
        let paid = Plan::from_parts("admin", Some(("razorpay", None)));
        assert_eq!(paid.detail(now, |_| String::new()), "Subscription.");
        assert_eq!(
            Plan::from_parts("user", Some(("future-thing", None))).label(),
            "Ottero Cloud"
        );
        for s in [CloudSource::Paid, CloudSource::Granted, CloudSource::Owner] {
            assert_eq!(CloudSource::parse(s.as_str()), Some(s));
        }
    }
}
