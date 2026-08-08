//! Account / login / usage app-server methods.
//!
//! Typed Codex app-server account methods:
//! - `account/read`
//! - `account/login/start`
//! - `account/login/cancel`
//! - `account/logout`
//! - `account/usage/read`
//! - `account/rateLimits/read`
//!
//! Includes account, login, usage, and rate-limit shapes.
//! `LoginAccount*.json`, `LogoutAccountResponse.json`,
//! `CancelLoginAccount*.json`, `GetAccountTokenUsageResponse.json`,
//! `GetAccountRateLimitsResponse.json`.
//!
//! Offline fixture returns a **demo** ChatGPT Pro profile (masked email),
//! usage numbers, and a device-code login stub **without network**.

use serde::{Deserialize, Serialize};
use std::fmt;

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

/// Plan tier from account / rate-limit payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanType {
    Free,
    Go,
    Plus,
    Pro,
    Prolite,
    Team,
    SelfServeBusinessUsageBased,
    Business,
    Ent26,
    EnterpriseCbpUsageBased,
    Enterprise,
    Edu,
    #[default]
    Unknown,
}

impl PlanType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Go => "go",
            Self::Plus => "plus",
            Self::Pro => "pro",
            Self::Prolite => "prolite",
            Self::Team => "team",
            Self::SelfServeBusinessUsageBased => "self_serve_business_usage_based",
            Self::Business => "business",
            Self::Ent26 => "ent26",
            Self::EnterpriseCbpUsageBased => "enterprise_cbp_usage_based",
            Self::Enterprise => "enterprise",
            Self::Edu => "edu",
            Self::Unknown => "unknown",
        }
    }

    /// Short display label for Settings chips.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plus => "Plus",
            Self::Pro => "Pro",
            Self::Free => "Free",
            Self::Team => "Team",
            Self::Business => "Business",
            Self::Enterprise | Self::Ent26 | Self::EnterpriseCbpUsageBased => "Enterprise",
            Self::Edu => "Edu",
            Self::Go => "Go",
            Self::Prolite => "Pro Lite",
            Self::SelfServeBusinessUsageBased => "Business (usage)",
            Self::Unknown => "Unknown",
        }
    }
}

impl fmt::Display for PlanType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

// ---------------------------------------------------------------------------
// account/read
// ---------------------------------------------------------------------------

/// Params for `account/read`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GetAccountParams {
    /// When true, request a proactive token refresh (managed auth only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<bool>,
}

/// Authenticated account object (wire `Account` oneOf).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Account {
    #[serde(rename = "apiKey")]
    ApiKey,
    #[serde(rename = "chatgpt")]
    Chatgpt {
        email: Option<String>,
        #[serde(rename = "planType")]
        plan_type: PlanType,
    },
    #[serde(rename = "amazonBedrock")]
    AmazonBedrock {
        #[serde(default, rename = "usesCodexManagedCredentials")]
        uses_codex_managed_credentials: bool,
    },
}

impl Account {
    /// True when this is a ChatGPT or API-key style account (usable auth).
    pub fn is_signed_in(&self) -> bool {
        matches!(self, Self::ApiKey | Self::Chatgpt { .. })
    }

    /// Masked email for UI (e.g. `d***@example.com`), if present.
    pub fn email_display(&self) -> Option<String> {
        match self {
            Self::Chatgpt { email: Some(e), .. } => Some(mask_email(e)),
            Self::Chatgpt { email: None, .. } => None,
            Self::ApiKey => Some("API key".into()),
            Self::AmazonBedrock { .. } => Some("Amazon Bedrock".into()),
        }
    }

    pub fn plan_type(&self) -> Option<PlanType> {
        match self {
            Self::Chatgpt { plan_type, .. } => Some(*plan_type),
            _ => None,
        }
    }
}

/// Response for `account/read`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GetAccountResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
    pub requires_openai_auth: bool,
}

impl GetAccountResponse {
    pub fn has_account(&self) -> bool {
        self.account.as_ref().is_some_and(|a| a.is_signed_in())
    }
}

// ---------------------------------------------------------------------------
// account/login/start + cancel + logout
// ---------------------------------------------------------------------------

/// Params for `account/login/start` (subset used by Mitsuro; full wire is a oneOf).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LoginAccountParams {
    #[serde(rename = "apiKey")]
    ApiKey {
        #[serde(rename = "apiKey")]
        api_key: String,
    },
    #[serde(rename = "chatgpt")]
    Chatgpt {
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "appBrand")]
        app_brand: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "codexStreamlinedLogin"
        )]
        codex_streamlined_login: Option<bool>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            rename = "useHostedLoginSuccessPage"
        )]
        use_hosted_login_success_page: Option<bool>,
    },
    /// Device-code flow: returns verification URL + user code (no browser OAuth redirect).
    #[serde(rename = "chatgptDeviceCode")]
    ChatgptDeviceCode,
}

impl LoginAccountParams {
    /// Fixture / UI default: device-code login (offline-safe stub).
    pub fn device_code() -> Self {
        Self::ChatgptDeviceCode
    }

    pub fn chatgpt() -> Self {
        Self::Chatgpt {
            app_brand: Some("codex".into()),
            codex_streamlined_login: None,
            use_hosted_login_success_page: None,
        }
    }
}

/// Response for `account/login/start`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LoginAccountResponse {
    #[serde(rename = "apiKey")]
    ApiKey,
    #[serde(rename = "chatgpt")]
    Chatgpt {
        #[serde(rename = "loginId")]
        login_id: String,
        /// URL the client should open to initiate OAuth.
        #[serde(rename = "authUrl")]
        auth_url: String,
    },
    #[serde(rename = "chatgptDeviceCode")]
    ChatgptDeviceCode {
        #[serde(rename = "loginId")]
        login_id: String,
        /// URL to open for device authorization.
        #[serde(rename = "verificationUrl")]
        verification_url: String,
        /// One-time code the user enters after signing in.
        #[serde(rename = "userCode")]
        user_code: String,
    },
    #[serde(rename = "chatgptAuthTokens")]
    ChatgptAuthTokens,
    #[serde(rename = "amazonBedrock")]
    AmazonBedrock,
}

impl LoginAccountResponse {
    /// Device URL if this is a device-code or OAuth start.
    pub fn device_url(&self) -> Option<&str> {
        match self {
            Self::ChatgptDeviceCode {
                verification_url, ..
            } => Some(verification_url.as_str()),
            Self::Chatgpt { auth_url, .. } => Some(auth_url.as_str()),
            _ => None,
        }
    }

    pub fn user_code(&self) -> Option<&str> {
        match self {
            Self::ChatgptDeviceCode { user_code, .. } => Some(user_code.as_str()),
            _ => None,
        }
    }

    pub fn login_id(&self) -> Option<&str> {
        match self {
            Self::Chatgpt { login_id, .. } | Self::ChatgptDeviceCode { login_id, .. } => {
                Some(login_id.as_str())
            }
            _ => None,
        }
    }
}

/// Params for `account/login/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CancelLoginAccountParams {
    pub login_id: String,
}

impl CancelLoginAccountParams {
    pub fn new(login_id: impl Into<String>) -> Self {
        Self {
            login_id: login_id.into(),
        }
    }
}

/// Status returned by `account/login/cancel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CancelLoginAccountStatus {
    Canceled,
    NotFound,
}

/// Response for `account/login/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CancelLoginAccountResponse {
    pub status: CancelLoginAccountStatus,
}

/// Response for `account/logout` (empty object on the wire).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogoutAccountResponse {}

// ---------------------------------------------------------------------------
// account/usage/read
// ---------------------------------------------------------------------------

/// Lifetime / streak summary from `account/usage/read`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountTokenUsageSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_daily_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longest_running_turn_sec: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_streak_days: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longest_streak_days: Option<i64>,
}

/// Daily bucket for token usage charts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountTokenUsageDailyBucket {
    pub start_date: String,
    pub tokens: i64,
}

/// Response for `account/usage/read`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GetAccountTokenUsageResponse {
    pub summary: AccountTokenUsageSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_usage_buckets: Option<Vec<AccountTokenUsageDailyBucket>>,
}

// ---------------------------------------------------------------------------
// account/rateLimits/read
// ---------------------------------------------------------------------------

/// A primary/secondary rate-limit window (`usedPercent` 0–100).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    pub used_percent: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_duration_mins: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

impl RateLimitWindow {
    pub fn remaining_percent(&self) -> i32 {
        (100 - self.used_percent).clamp(0, 100)
    }
}

/// Credits snapshot nested under rate limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
}

/// Rate-limit snapshot (primary/secondary windows + plan).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<RateLimitWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<RateLimitWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<CreditsSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<PlanType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_control_reached: Option<bool>,
}

/// Response for `account/rateLimits/read`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GetAccountRateLimitsResponse {
    pub rate_limits: RateLimitSnapshot,
}

// ---------------------------------------------------------------------------
// Fixture demo payloads
// ---------------------------------------------------------------------------

/// Masked demo email shown in fixture mode (`d***@example.com`).
pub const FIXTURE_DEMO_EMAIL_MASKED: &str = "d***@example.com";

/// Full demo email before masking (not shown in UI by default).
pub const FIXTURE_DEMO_EMAIL: &str = "demo@example.com";

/// Sidebar / settings display name for the offline fixture profile.
pub const FIXTURE_DEMO_DISPLAY_NAME: &str = "Jacob Burgess";

/// Fixture device verification URL (no network; display-only stub).
pub const FIXTURE_LOGIN_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";

/// Fixture one-time user code for device login stub.
pub const FIXTURE_LOGIN_USER_CODE: &str = "MITSU-DEMO";

/// Fixture login id returned by `account/login/start`.
pub const FIXTURE_LOGIN_ID: &str = "fixture-login-1";

/// Mask an email for display: keep first char of local + domain.
pub fn mask_email(email: &str) -> String {
    let email = email.trim();
    if email.is_empty() {
        return String::new();
    }
    let Some((local, domain)) = email.split_once('@') else {
        // Not an email — show first char + ***
        let mut chars = email.chars();
        let first = chars.next().unwrap_or('?');
        return format!("{first}***");
    };
    let first = local.chars().next().unwrap_or('?');
    format!("{first}***@{domain}")
}

/// Demo ChatGPT Pro account (email already suitable for wire; UI should mask).
pub fn fixture_demo_account() -> Account {
    Account::Chatgpt {
        // Wire may carry a real-looking email; UI uses [`Account::email_display`].
        email: Some(FIXTURE_DEMO_EMAIL.into()),
        plan_type: PlanType::Pro,
    }
}

/// `account/read` when fixture is signed in.
pub fn fixture_demo_account_response() -> GetAccountResponse {
    GetAccountResponse {
        account: Some(fixture_demo_account()),
        requires_openai_auth: true,
    }
}

/// `account/read` when fixture is signed out.
pub fn fixture_signed_out_account_response() -> GetAccountResponse {
    GetAccountResponse {
        account: None,
        requires_openai_auth: true,
    }
}

/// Demo token usage numbers for Settings bars.
pub fn fixture_demo_usage() -> GetAccountTokenUsageResponse {
    GetAccountTokenUsageResponse {
        summary: AccountTokenUsageSummary {
            lifetime_tokens: Some(12_450_000),
            peak_daily_tokens: Some(480_000),
            longest_running_turn_sec: Some(312),
            current_streak_days: Some(4),
            longest_streak_days: Some(14),
        },
        daily_usage_buckets: Some(vec![
            AccountTokenUsageDailyBucket {
                start_date: "2026-08-01".into(),
                tokens: 210_000,
            },
            AccountTokenUsageDailyBucket {
                start_date: "2026-08-02".into(),
                tokens: 340_000,
            },
            AccountTokenUsageDailyBucket {
                start_date: "2026-08-03".into(),
                tokens: 480_000,
            },
            AccountTokenUsageDailyBucket {
                start_date: "2026-08-04".into(),
                tokens: 125_000,
            },
        ]),
    }
}

/// Demo rate limits (primary 5h / secondary weekly style windows).
pub fn fixture_demo_rate_limits() -> GetAccountRateLimitsResponse {
    GetAccountRateLimitsResponse {
        rate_limits: RateLimitSnapshot {
            limit_id: Some("codex".into()),
            limit_name: Some("Codex".into()),
            primary: Some(RateLimitWindow {
                used_percent: 42,
                window_duration_mins: Some(300), // 5h
                resets_at: Some(1_754_323_200),  // synthetic
            }),
            secondary: Some(RateLimitWindow {
                used_percent: 18,
                window_duration_mins: Some(10_080), // 7d
                resets_at: Some(1_754_755_200),
            }),
            credits: Some(CreditsSnapshot {
                has_credits: false,
                unlimited: false,
                balance: None,
            }),
            plan_type: Some(PlanType::Pro),
            spend_control_reached: Some(false),
        },
    }
}

/// Device-code login stub (no network).
pub fn fixture_login_device_code_response() -> LoginAccountResponse {
    LoginAccountResponse::ChatgptDeviceCode {
        login_id: FIXTURE_LOGIN_ID.into(),
        verification_url: FIXTURE_LOGIN_VERIFICATION_URL.into(),
        user_code: FIXTURE_LOGIN_USER_CODE.into(),
    }
}

/// ChatGPT OAuth-style start stub (auth URL only; still no network).
pub fn fixture_login_chatgpt_response() -> LoginAccountResponse {
    LoginAccountResponse::Chatgpt {
        login_id: FIXTURE_LOGIN_ID.into(),
        auth_url: format!("{FIXTURE_LOGIN_VERIFICATION_URL}?client=mitsuro"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_email_keeps_domain() {
        assert_eq!(mask_email("demo@example.com"), "d***@example.com");
        assert_eq!(mask_email("ab@x.co"), "a***@x.co");
        assert_eq!(mask_email(""), "");
    }

    #[test]
    fn fixture_demo_account_is_pro_chatgpt() {
        let resp = fixture_demo_account_response();
        assert!(resp.has_account());
        assert!(resp.requires_openai_auth);
        let acc = resp.account.unwrap();
        assert_eq!(acc.plan_type(), Some(PlanType::Pro));
        assert_eq!(
            acc.email_display().as_deref(),
            Some(FIXTURE_DEMO_EMAIL_MASKED)
        );
        assert_eq!(FIXTURE_DEMO_DISPLAY_NAME, "Jacob Burgess");
    }

    #[test]
    fn fixture_usage_has_numbers() {
        let u = fixture_demo_usage();
        assert!(u.summary.lifetime_tokens.unwrap() > 0);
        assert_eq!(u.daily_usage_buckets.as_ref().unwrap().len(), 4);
    }

    #[test]
    fn fixture_rate_limits_used_percent() {
        let r = fixture_demo_rate_limits();
        let p = r.rate_limits.primary.as_ref().unwrap();
        assert_eq!(p.used_percent, 42);
        assert_eq!(p.remaining_percent(), 58);
        assert_eq!(r.rate_limits.secondary.as_ref().unwrap().used_percent, 18);
        assert_eq!(r.rate_limits.plan_type, Some(PlanType::Pro));
    }

    #[test]
    fn login_device_code_stub_has_url_and_code() {
        let r = fixture_login_device_code_response();
        assert_eq!(r.device_url(), Some(FIXTURE_LOGIN_VERIFICATION_URL));
        assert_eq!(r.user_code(), Some(FIXTURE_LOGIN_USER_CODE));
        assert_eq!(r.login_id(), Some(FIXTURE_LOGIN_ID));
    }

    #[test]
    fn serialize_account_read_camel_case() {
        let v = serde_json::to_value(fixture_demo_account_response()).unwrap();
        assert!(v.get("requiresOpenaiAuth").is_some());
        assert_eq!(v["account"]["type"], "chatgpt");
        assert_eq!(v["account"]["planType"], "pro");
        assert_eq!(v["account"]["email"], FIXTURE_DEMO_EMAIL);
    }

    #[test]
    fn serialize_login_device_code_camel_case() {
        let v = serde_json::to_value(fixture_login_device_code_response()).unwrap();
        assert_eq!(v["type"], "chatgptDeviceCode");
        assert_eq!(v["loginId"], FIXTURE_LOGIN_ID);
        assert_eq!(v["userCode"], FIXTURE_LOGIN_USER_CODE);
        assert_eq!(v["verificationUrl"], FIXTURE_LOGIN_VERIFICATION_URL);
    }

    #[test]
    fn serialize_usage_and_rate_limits_camel_case() {
        let u = serde_json::to_value(fixture_demo_usage()).unwrap();
        assert!(u["summary"]["lifetimeTokens"].as_i64().unwrap() > 0);
        let r = serde_json::to_value(fixture_demo_rate_limits()).unwrap();
        assert_eq!(r["rateLimits"]["primary"]["usedPercent"], 42);
        assert_eq!(r["rateLimits"]["planType"], "pro");
    }

    #[test]
    fn deserialize_login_params_device_code() {
        let p: LoginAccountParams =
            serde_json::from_value(serde_json::json!({"type": "chatgptDeviceCode"})).unwrap();
        assert_eq!(p, LoginAccountParams::ChatgptDeviceCode);
    }

    #[test]
    fn cancel_and_logout_shapes() {
        let c = CancelLoginAccountResponse {
            status: CancelLoginAccountStatus::Canceled,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["status"], "canceled");
        let empty = serde_json::to_value(LogoutAccountResponse::default()).unwrap();
        assert!(empty.as_object().unwrap().is_empty() || empty == serde_json::json!({}));
    }
}
