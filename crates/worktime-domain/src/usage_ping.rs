//! The anonymous daily usage signal: whether one is due today, and what it holds. Its contract
//! with the service is `services/worktime/src/ping.ts`.

use jiff::civil::Date;

/// Where WorkTime was installed from, fixed at build time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Store,
    /// Installed by the Velopack `Setup.exe`, which updates itself from WorkTime's feed.
    Direct,
    Zip,
}

impl Channel {
    /// The value the service accepts.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Direct => "direct",
            Self::Zip => "zip",
        }
    }
}

/// The build that sends the signal. The version is one the service accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Build {
    version: &'static str,
    channel: Channel,
}

impl Build {
    /// `None` unless `version` matches `^\d{1,3}\.\d{1,3}\.\d{1,3}(-[0-9A-Za-z.]{1,20})?$`, the
    /// only versions the service records, so the signal never carries free text.
    pub fn new(version: &'static str, channel: Channel) -> Option<Self> {
        let (release, prerelease) = match version.split_once('-') {
            Some((release, prerelease)) => (release, Some(prerelease)),
            None => (version, None),
        };
        let numbers: Vec<&str> = release.split('.').collect();
        let release_ok = numbers.len() == 3
            && numbers.iter().all(|number| {
                (1..=3).contains(&number.len()) && number.bytes().all(|byte| byte.is_ascii_digit())
            });
        let prerelease_ok = prerelease.is_none_or(|prerelease| {
            (1..=20).contains(&prerelease.len())
                && prerelease
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
        });
        (release_ok && prerelease_ok).then_some(Self { version, channel })
    }

    pub fn version(self) -> &'static str {
        self.version
    }

    pub fn channel(self) -> Channel {
        self.channel
    }
}

/// The whole signal. Nothing else is ever sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsagePing {
    pub version: &'static str,
    pub channel: Channel,
    pub first_this_month: bool,
}

/// The signal to send today, or `None` when it is turned off, was already sent today, or the
/// clock went back before the last send. `today` and `last_sent` are local dates; the first
/// signal of a calendar month says so.
pub fn usage_ping_due(
    enabled: bool,
    last_sent: Option<Date>,
    today: Date,
    build: Build,
) -> Option<UsagePing> {
    if !enabled || last_sent.is_some_and(|last_sent| last_sent >= today) {
        return None;
    }
    let first_this_month = last_sent.is_none_or(|last_sent| {
        (last_sent.year(), last_sent.month()) != (today.year(), today.month())
    });
    Some(UsagePing {
        version: build.version,
        channel: build.channel,
        first_this_month,
    })
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::*;

    const TODAY: Date = date(2026, 10, 13);

    fn build() -> Build {
        Build::new("1.0.0", Channel::Store).unwrap()
    }

    fn due(enabled: bool, last_sent: Option<Date>) -> Option<UsagePing> {
        usage_ping_due(enabled, last_sent, TODAY, build())
    }

    fn ping(first_this_month: bool) -> Option<UsagePing> {
        Some(UsagePing {
            version: "1.0.0",
            channel: Channel::Store,
            first_this_month,
        })
    }

    #[test]
    fn a_build_that_never_sent_sends_the_first_signal_of_the_month() {
        assert_eq!(due(true, None), ping(true));
    }

    #[test]
    fn nothing_is_due_once_sent_today() {
        assert_eq!(due(true, Some(TODAY)), None);
    }

    #[test]
    fn a_signal_sent_yesterday_in_the_same_month_makes_today_an_ordinary_day() {
        assert_eq!(due(true, Some(date(2026, 10, 12))), ping(false));
    }

    #[test]
    fn a_signal_sent_last_month_makes_today_the_first_of_the_month() {
        assert_eq!(due(true, Some(date(2026, 9, 30))), ping(true));
    }

    #[test]
    fn a_signal_sent_in_the_same_month_of_last_year_makes_today_the_first_of_the_month() {
        assert_eq!(due(true, Some(date(2025, 10, 13))), ping(true));
    }

    #[test]
    fn a_signal_sent_last_december_makes_january_the_first_of_the_month() {
        let today = date(2027, 1, 2);

        assert_eq!(
            usage_ping_due(true, Some(date(2026, 12, 31)), today, build()),
            ping(true)
        );
    }

    #[test]
    fn nothing_is_due_when_the_setting_is_off() {
        assert_eq!(due(false, None), None);
        assert_eq!(due(false, Some(date(2026, 10, 12))), None);
    }

    #[test]
    fn nothing_is_due_when_the_clock_went_back_before_the_last_send() {
        assert_eq!(due(true, Some(date(2026, 10, 14))), None);
    }

    #[test]
    fn the_signal_carries_the_build_version_and_channel() {
        let build = Build::new("1.2.3-alpha.4", Channel::Zip).unwrap();

        assert_eq!(
            usage_ping_due(true, None, TODAY, build),
            Some(UsagePing {
                version: "1.2.3-alpha.4",
                channel: Channel::Zip,
                first_this_month: true,
            })
        );
    }

    #[test]
    fn only_versions_the_service_accepts_make_a_build() {
        for accepted in [
            "0.1.0",
            "1.0.0",
            "999.999.999",
            "1.0.0-a",
            "1.0.0-alpha.3",
            "1.0.0-12345678901234567890",
        ] {
            assert!(Build::new(accepted, Channel::Store).is_some(), "{accepted}");
        }
        for rejected in [
            "",
            "1.0",
            "1.0.0.0",
            "1000.0.0",
            "1..0",
            "a.0.0",
            "1.0.0-",
            "1.0.0-123456789012345678901",
            "1.0.0-alpha_3",
            "1.0.0-a-b",
            "1.0.0+build",
            "1.0.0-a\"b",
            "１.0.0",
        ] {
            assert_eq!(Build::new(rejected, Channel::Store), None, "{rejected}");
        }
    }

    #[test]
    fn channels_use_the_service_spelling() {
        assert_eq!(Channel::Store.as_str(), "store");
        assert_eq!(Channel::Direct.as_str(), "direct");
        assert_eq!(Channel::Zip.as_str(), "zip");
    }
}
