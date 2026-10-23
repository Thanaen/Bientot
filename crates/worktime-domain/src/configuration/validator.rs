use jiff::SignedDuration;

use super::{
    ActivityWatchSettings, ActivityWatchSettlementContract, BuiltInActivitySettings,
    LoggingSettings, NotificationRule, RecurringAppointment, WorkTimeSettings,
};

/// A rejected group of settings. [`SettingsError::resource_key`] names the message shown for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsError {
    Schema,
    Target,
    Minimum,
    Lunch,
    Departure,
    Margin,
    Lock,
    Debt,
    Language,
    Source,
    IdleThreshold,
    Notifications,
    Storage,
    Appointments,
}

impl SettingsError {
    pub fn resource_key(self) -> &'static str {
        match self {
            Self::Schema => "SettingsSchema",
            Self::Target => "SettingsTarget",
            Self::Minimum => "SettingsMinimum",
            Self::Lunch => "SettingsLunch",
            Self::Departure => "SettingsDeparture",
            Self::Margin => "SettingsMargin",
            Self::Lock => "SettingsLock",
            Self::Debt => "SettingsDebt",
            Self::Language => "SettingsLanguage",
            Self::Source => "SettingsSource",
            Self::IdleThreshold => "SettingsIdleThreshold",
            Self::Notifications => "SettingsNotifications",
            Self::Storage => "SettingsStorage",
            Self::Appointments => "SettingsAppointments",
        }
    }
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Schema => "the settings schema version is not supported",
            Self::Target => "the daily target must be positive and at most 24 hours",
            Self::Minimum => {
                "the minimum effective target must be positive and at most the daily target"
            }
            Self::Lunch => "lunch must end after it starts",
            Self::Departure => "the preferred latest departure must be after lunch",
            Self::Margin => "the warning margin must be between zero and 24 hours",
            Self::Lock => "the long-lock review threshold must be positive and at most 24 hours",
            Self::Debt => "the weekly debt limit must be positive and at most 24 hours",
            Self::Language => "the language must be fr-FR or en-US",
            Self::Source => "the ActivityWatch address or timings cannot be used",
            Self::IdleThreshold => "the idle threshold must be between 30 seconds and 30 minutes",
            Self::Notifications => "notification lead times must be between zero and 24 hours",
            Self::Storage => "the log file limits are out of range",
            Self::Appointments => {
                "a recurring appointment has no usable identifier or name, falls on a weekend or \
                 ends before it starts"
            }
        })
    }
}

impl std::error::Error for SettingsError {}

const LINEAR_WHITESPACE: [char; 4] = [' ', '\t', '\r', '\n'];

const DAY: SignedDuration = SignedDuration::from_hours(24);

impl WorkTimeSettings {
    /// Every rejected group, in a stable order; empty when the settings can be used.
    pub fn validate(&self) -> Vec<SettingsError> {
        let checks = [
            (
                self.schema_version == Self::CURRENT_SCHEMA_VERSION,
                SettingsError::Schema,
            ),
            (
                within(self.nominal_daily_target, DAY),
                SettingsError::Target,
            ),
            (
                within(self.minimum_effective_target, self.nominal_daily_target),
                SettingsError::Minimum,
            ),
            (self.lunch_end > self.lunch_start, SettingsError::Lunch),
            (
                self.preferred_latest_departure > self.lunch_end,
                SettingsError::Departure,
            ),
            (
                !self.orange_warning_margin.is_negative() && self.orange_warning_margin <= DAY,
                SettingsError::Margin,
            ),
            (
                within(self.long_lock_review_threshold, DAY),
                SettingsError::Lock,
            ),
            (
                self.maximum_weekly_debt
                    .is_none_or(|maximum| within(maximum, DAY)),
                SettingsError::Debt,
            ),
            (
                matches!(
                    self.language_override.as_deref(),
                    None | Some("fr-FR" | "en-US")
                ),
                SettingsError::Language,
            ),
            (valid_source(&self.activity_watch), SettingsError::Source),
            (
                valid_idle_threshold(self.built_in_activity),
                SettingsError::IdleThreshold,
            ),
            (
                [
                    self.notifications.completion_approach,
                    self.notifications.latest_departure_approach,
                    self.notifications.day_complete,
                ]
                .into_iter()
                .all(valid_rule),
                SettingsError::Notifications,
            ),
            (valid_logging(self.logging), SettingsError::Storage),
            (
                valid_appointments(&self.recurring_appointments),
                SettingsError::Appointments,
            ),
        ];
        checks
            .into_iter()
            .filter(|(valid, _)| !valid)
            .map(|(_, error)| error)
            .collect()
    }
}

/// Identifiers name reviews and adjustments, so they keep to `[A-Za-z0-9-]` and are unique.
fn valid_appointments(appointments: &[RecurringAppointment]) -> bool {
    let mut ids = std::collections::HashSet::new();
    appointments.len() <= RecurringAppointment::MAXIMUM_COUNT
        && appointments.iter().all(|appointment| {
            (1..=RecurringAppointment::MAXIMUM_ID_LENGTH).contains(&appointment.id.len())
                && appointment
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && ids.insert(appointment.id.as_str())
                && appointment.name.chars().count() <= RecurringAppointment::MAXIMUM_NAME_LENGTH
                && !appointment.name.chars().any(char::is_control)
                && appointment.weekday.to_monday_one_offset() <= 5
                && appointment.end > appointment.start
                && appointment
                    .alternate_weeks_from
                    .is_none_or(|from| from.weekday() == appointment.weekday)
        })
}

fn within(duration: SignedDuration, maximum: SignedDuration) -> bool {
    duration.is_positive() && duration <= maximum
}

fn valid_rule(rule: NotificationRule) -> bool {
    !rule.lead_time.is_negative() && rule.lead_time <= DAY
}

fn valid_idle_threshold(settings: BuiltInActivitySettings) -> bool {
    (BuiltInActivitySettings::MINIMUM_IDLE_THRESHOLD
        ..=BuiltInActivitySettings::MAXIMUM_IDLE_THRESHOLD)
        .contains(&settings.idle_threshold)
}

/// Same bounds as the diagnostic log options, so saved limits never fail the logger at startup.
fn valid_logging(logging: LoggingSettings) -> bool {
    logging.maximum_file_count > 0
        && (4_096..=20 * 1024 * 1024).contains(&logging.maximum_file_bytes)
}

fn valid_source(source: &ActivityWatchSettings) -> bool {
    is_usable_base_address(&source.base_url)
        && within(source.request_timeout, SignedDuration::from_mins(1))
        && valid_settlement(source.settlement)
}

fn valid_settlement(settlement: ActivityWatchSettlementContract) -> bool {
    let half_hour = SignedDuration::from_mins(30);
    within(settlement.afk_timeout, half_hour)
        && within(settlement.poll_interval, half_hour)
        && !settlement.maximum_jitter.is_negative()
        && settlement.maximum_jitter <= SignedDuration::from_mins(5)
        && settlement.horizon() <= half_hour
}

/// An absolute `http` or `https` address of a host root, without credentials, query or fragment.
/// Follows the .NET `Uri` parser the app used on the inputs a person plausibly types.
fn is_usable_base_address(address: &str) -> bool {
    if address.contains(['?', '#']) {
        return false;
    }
    let address = address.trim_start_matches(LINEAR_WHITESPACE);
    let trimmed = address.trim_end_matches(LINEAR_WHITESPACE);
    let decode_dots = trimmed.len() == address.len();
    let address = trimmed;
    let Some((scheme, rest)) = address.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let (authority, path) = rest.find('/').map_or((rest, ""), |at| rest.split_at(at));
    if authority.contains('\\') {
        return false;
    }
    let host_and_port = match authority.rsplit_once('@') {
        Some(("", host_and_port)) => host_and_port,
        Some(_) => return false,
        None => authority,
    };
    let (host, port) = split_port(host_and_port);
    valid_host(host)
        && port.is_none_or(valid_port)
        && is_root_path(&path.replace('\\', "/"), decode_dots)
}

fn split_port(host_and_port: &str) -> (&str, Option<&str>) {
    if host_and_port.starts_with('[') {
        return match host_and_port.find(']') {
            Some(end) => match host_and_port[end + 1..].strip_prefix(':') {
                Some(port) => (&host_and_port[..=end], Some(port)),
                None if end + 1 == host_and_port.len() => (host_and_port, None),
                None => ("", None),
            },
            None => ("", None),
        };
    }
    match host_and_port.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (host_and_port, None),
    }
}

fn valid_port(port: &str) -> bool {
    port.is_empty()
        || (port.bytes().all(|byte| byte.is_ascii_digit())
            && port.parse::<u32>().is_ok_and(|port| port <= 65_535))
}

fn valid_host(host: &str) -> bool {
    if let Some(literal) = host.strip_prefix('[') {
        let Some(literal) = literal.strip_suffix(']') else {
            return false;
        };
        let address = literal
            .split_once("%25")
            .map_or(literal, |(address, _)| address);
        return address.parse::<std::net::Ipv6Addr>().is_ok();
    }
    let labels = host.strip_suffix('.').unwrap_or(host);
    !labels.is_empty()
        && labels.split('.').all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || !c.is_ascii() || c == '-' || c == '_')
        })
}

/// Whether the path resolves to `/` once its dot segments are removed. Percent-encoded dots count
/// only when no trailing whitespace was trimmed, as in .NET.
fn is_root_path(path: &str, decode_dots: bool) -> bool {
    let mut depth = 0usize;
    let segments: Vec<_> = path.split('/').skip(1).collect();
    for (index, segment) in segments.iter().enumerate() {
        let last = index + 1 == segments.len();
        match dot_segment(segment, decode_dots).as_deref() {
            Some(".") => {}
            Some("..") => depth = depth.saturating_sub(1),
            _ if segment.is_empty() && last => {}
            _ => depth += 1,
        }
    }
    depth == 0
}

fn dot_segment(segment: &str, decode: bool) -> Option<String> {
    let decoded = if decode {
        segment.replace("%2e", ".").replace("%2E", ".")
    } else {
        segment.to_owned()
    };
    decoded.chars().all(|c| c == '.').then_some(decoded)
}

#[cfg(test)]
mod tests;
