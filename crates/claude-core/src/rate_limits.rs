use once_cell::sync::Lazy;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    Allowed,
    AllowedWarning,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitType {
    FiveHour,
    SevenDay,
    SevenDayOpus,
    SevenDaySonnet,
    Overage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverageDisabledReason {
    OverageNotProvisioned,
    OrgLevelDisabled,
    OrgLevelDisabledUntil,
    OutOfCredits,
    SeatTierLevelDisabled,
    MemberLevelDisabled,
    SeatTierZeroCreditLimit,
    GroupZeroCreditLimit,
    MemberZeroCreditLimit,
    OrgServiceLevelDisabled,
    OrgServiceZeroCreditLimit,
    NoLimitsConfigured,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAiLimits {
    pub status: QuotaStatus,
    #[serde(skip_serializing)]
    pub unified_rate_limit_fallback_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_type: Option<RateLimitType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_status: Option<QuotaStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_resets_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overage_disabled_reason: Option<OverageDisabledReason>,
    pub is_using_overage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surpassed_threshold: Option<f64>,
}

impl Default for ClaudeAiLimits {
    fn default() -> Self {
        Self {
            status: QuotaStatus::Allowed,
            unified_rate_limit_fallback_available: false,
            resets_at: None,
            rate_limit_type: None,
            utilization: None,
            overage_status: None,
            overage_resets_at: None,
            overage_disabled_reason: None,
            is_using_overage: false,
            surpassed_threshold: None,
        }
    }
}

type StatusChangeListener = Arc<dyn Fn(ClaudeAiLimits) + Send + Sync + 'static>;

static CURRENT_LIMITS: Lazy<Mutex<ClaudeAiLimits>> =
    Lazy::new(|| Mutex::new(ClaudeAiLimits::default()));
static LISTENERS: Lazy<Mutex<HashMap<u64, StatusChangeListener>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_LISTENER_ID: AtomicU64 = AtomicU64::new(1);

const EARLY_WARNING_CONFIGS: &[(RateLimitType, &str, f64, &[(f64, f64)])] = &[
    (
        RateLimitType::FiveHour,
        "5h",
        5.0 * 60.0 * 60.0,
        &[(0.9, 0.72)],
    ),
    (
        RateLimitType::SevenDay,
        "7d",
        7.0 * 24.0 * 60.0 * 60.0,
        &[(0.75, 0.6), (0.5, 0.35), (0.25, 0.15)],
    ),
];

pub fn register_status_listener(listener: StatusChangeListener) -> u64 {
    let id = NEXT_LISTENER_ID.fetch_add(1, Ordering::SeqCst);
    LISTENERS.lock().unwrap().insert(id, listener);
    id
}

pub fn unregister_status_listener(id: u64) {
    LISTENERS.lock().unwrap().remove(&id);
}

pub fn current_limits() -> ClaudeAiLimits {
    CURRENT_LIMITS.lock().unwrap().clone()
}

pub fn reset_for_tests() {
    *CURRENT_LIMITS.lock().unwrap() = ClaudeAiLimits::default();
    LISTENERS.lock().unwrap().clear();
}

pub fn extract_quota_status_from_headers(headers: &HeaderMap, is_subscriber: bool) {
    if !should_process_rate_limits(is_subscriber) {
        let mut current = CURRENT_LIMITS.lock().unwrap();
        if *current != ClaudeAiLimits::default() {
            *current = ClaudeAiLimits::default();
            emit_to_listeners(current.clone());
        }
        return;
    }

    let new_limits = compute_new_limits_from_headers(headers);
    emit_status_change_if_needed(new_limits);
}

pub fn extract_quota_status_from_error(
    headers: Option<&HeaderMap>,
    status: u16,
    is_subscriber: bool,
) {
    if !should_process_rate_limits(is_subscriber) || status != 429 {
        return;
    }

    let mut new_limits = headers
        .map(compute_new_limits_from_headers)
        .unwrap_or_else(current_limits);
    new_limits.status = QuotaStatus::Rejected;
    emit_status_change_if_needed(new_limits);
}

fn should_process_rate_limits(is_subscriber: bool) -> bool {
    is_subscriber
}

fn emit_status_change_if_needed(new_limits: ClaudeAiLimits) {
    let mut current = CURRENT_LIMITS.lock().unwrap();
    if *current == new_limits {
        return;
    }
    *current = new_limits.clone();
    drop(current);
    emit_to_listeners(new_limits);
}

fn emit_to_listeners(limits: ClaudeAiLimits) {
    let listeners = LISTENERS
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for listener in listeners {
        listener(limits.clone());
    }
}

fn compute_new_limits_from_headers(headers: &HeaderMap) -> ClaudeAiLimits {
    let status = header_quota_status(headers, "anthropic-ratelimit-unified-status")
        .unwrap_or(QuotaStatus::Allowed);
    let resets_at = header_f64(headers, "anthropic-ratelimit-unified-reset");
    let unified_rate_limit_fallback_available =
        header_str(headers, "anthropic-ratelimit-unified-fallback").as_deref() == Some("available");
    let rate_limit_type =
        header_rate_limit_type(headers, "anthropic-ratelimit-unified-representative-claim");
    let overage_status = header_quota_status(headers, "anthropic-ratelimit-unified-overage-status");
    let overage_resets_at = header_f64(headers, "anthropic-ratelimit-unified-overage-reset");
    let overage_disabled_reason = header_overage_disabled_reason(
        headers,
        "anthropic-ratelimit-unified-overage-disabled-reason",
    );
    let is_using_overage = matches!(status, QuotaStatus::Rejected)
        && matches!(
            overage_status,
            Some(QuotaStatus::Allowed | QuotaStatus::AllowedWarning)
        );

    if matches!(status, QuotaStatus::Allowed | QuotaStatus::AllowedWarning) {
        if let Some(warning) =
            early_warning_from_headers(headers, unified_rate_limit_fallback_available)
        {
            return warning;
        }
    }

    ClaudeAiLimits {
        status: if matches!(status, QuotaStatus::Allowed | QuotaStatus::AllowedWarning) {
            QuotaStatus::Allowed
        } else {
            status
        },
        unified_rate_limit_fallback_available,
        resets_at,
        rate_limit_type,
        utilization: None,
        overage_status,
        overage_resets_at,
        overage_disabled_reason,
        is_using_overage,
        surpassed_threshold: None,
    }
}

fn early_warning_from_headers(
    headers: &HeaderMap,
    unified_rate_limit_fallback_available: bool,
) -> Option<ClaudeAiLimits> {
    for (claim_abbrev, rate_limit_type) in [
        ("5h", RateLimitType::FiveHour),
        ("7d", RateLimitType::SevenDay),
        ("overage", RateLimitType::Overage),
    ] {
        let surpassed_threshold = header_f64(
            headers,
            &format!("anthropic-ratelimit-unified-{claim_abbrev}-surpassed-threshold"),
        );
        if let Some(surpassed_threshold) = surpassed_threshold {
            return Some(ClaudeAiLimits {
                status: QuotaStatus::AllowedWarning,
                resets_at: header_f64(
                    headers,
                    &format!("anthropic-ratelimit-unified-{claim_abbrev}-reset"),
                ),
                rate_limit_type: Some(rate_limit_type),
                utilization: header_f64(
                    headers,
                    &format!("anthropic-ratelimit-unified-{claim_abbrev}-utilization"),
                ),
                unified_rate_limit_fallback_available,
                is_using_overage: false,
                surpassed_threshold: Some(surpassed_threshold),
                ..ClaudeAiLimits::default()
            });
        }
    }

    for (rate_limit_type, claim_abbrev, window_seconds, thresholds) in EARLY_WARNING_CONFIGS {
        let utilization = header_f64(
            headers,
            &format!("anthropic-ratelimit-unified-{claim_abbrev}-utilization"),
        )?;
        let resets_at = header_f64(
            headers,
            &format!("anthropic-ratelimit-unified-{claim_abbrev}-reset"),
        )?;
        let time_progress = compute_time_progress(resets_at, *window_seconds);
        if thresholds.iter().any(|(utilization_threshold, time_pct)| {
            utilization >= *utilization_threshold && time_progress <= *time_pct
        }) {
            return Some(ClaudeAiLimits {
                status: QuotaStatus::AllowedWarning,
                resets_at: Some(resets_at),
                rate_limit_type: Some(rate_limit_type.clone()),
                utilization: Some(utilization),
                unified_rate_limit_fallback_available,
                is_using_overage: false,
                ..ClaudeAiLimits::default()
            });
        }
    }

    None
}

fn compute_time_progress(resets_at: f64, window_seconds: f64) -> f64 {
    let now_seconds = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
    let window_start = resets_at - window_seconds;
    ((now_seconds - window_start) / window_seconds).clamp(0.0, 1.0)
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn header_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
    header_str(headers, name).and_then(|value| value.parse::<f64>().ok())
}

fn header_quota_status(headers: &HeaderMap, name: &str) -> Option<QuotaStatus> {
    match header_str(headers, name)?.as_str() {
        "allowed" => Some(QuotaStatus::Allowed),
        "allowed_warning" => Some(QuotaStatus::AllowedWarning),
        "rejected" => Some(QuotaStatus::Rejected),
        _ => None,
    }
}

fn header_rate_limit_type(headers: &HeaderMap, name: &str) -> Option<RateLimitType> {
    match header_str(headers, name)?.as_str() {
        "five_hour" => Some(RateLimitType::FiveHour),
        "seven_day" => Some(RateLimitType::SevenDay),
        "seven_day_opus" => Some(RateLimitType::SevenDayOpus),
        "seven_day_sonnet" => Some(RateLimitType::SevenDaySonnet),
        "overage" => Some(RateLimitType::Overage),
        _ => None,
    }
}

fn header_overage_disabled_reason(
    headers: &HeaderMap,
    name: &str,
) -> Option<OverageDisabledReason> {
    match header_str(headers, name)?.as_str() {
        "overage_not_provisioned" => Some(OverageDisabledReason::OverageNotProvisioned),
        "org_level_disabled" => Some(OverageDisabledReason::OrgLevelDisabled),
        "org_level_disabled_until" => Some(OverageDisabledReason::OrgLevelDisabledUntil),
        "out_of_credits" => Some(OverageDisabledReason::OutOfCredits),
        "seat_tier_level_disabled" => Some(OverageDisabledReason::SeatTierLevelDisabled),
        "member_level_disabled" => Some(OverageDisabledReason::MemberLevelDisabled),
        "seat_tier_zero_credit_limit" => Some(OverageDisabledReason::SeatTierZeroCreditLimit),
        "group_zero_credit_limit" => Some(OverageDisabledReason::GroupZeroCreditLimit),
        "member_zero_credit_limit" => Some(OverageDisabledReason::MemberZeroCreditLimit),
        "org_service_level_disabled" => Some(OverageDisabledReason::OrgServiceLevelDisabled),
        "org_service_zero_credit_limit" => Some(OverageDisabledReason::OrgServiceZeroCreditLimit),
        "no_limits_configured" => Some(OverageDisabledReason::NoLimitsConfigured),
        "unknown" => Some(OverageDisabledReason::Unknown),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};
    use std::sync::{Arc, Mutex};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn headers(values: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in values {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn emits_status_change_from_success_headers() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        register_status_listener(Arc::new(move |limits| {
            captured.lock().unwrap().push(limits);
        }));

        extract_quota_status_from_headers(
            &headers(&[
                ("anthropic-ratelimit-unified-status", "allowed"),
                (
                    "anthropic-ratelimit-unified-representative-claim",
                    "seven_day",
                ),
                ("anthropic-ratelimit-unified-reset", "123"),
            ]),
            true,
        );

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, QuotaStatus::Allowed);
        assert_eq!(events[0].rate_limit_type, Some(RateLimitType::SevenDay));
        assert_eq!(events[0].resets_at, Some(123.0));
    }

    #[test]
    fn emits_rejected_on_headerless_429() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        register_status_listener(Arc::new(move |limits| {
            captured.lock().unwrap().push(limits);
        }));

        extract_quota_status_from_error(None, 429, true);

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, QuotaStatus::Rejected);
    }

    #[test]
    fn skips_non_subscribers_without_mock_limits() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_for_tests();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        register_status_listener(Arc::new(move |limits| {
            captured.lock().unwrap().push(limits);
        }));

        extract_quota_status_from_headers(
            &headers(&[("anthropic-ratelimit-unified-status", "rejected")]),
            false,
        );

        assert!(events.lock().unwrap().is_empty());
    }
}
