use jiff::Timestamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceFreshness {
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceQuality {
    Complete,
    Incomplete,
    Ambiguous,
    Unauthorized,
    Incompatible,
    Malformed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceHealth {
    pub source_id: String,
    pub freshness: SourceFreshness,
    pub quality: SourceQuality,
    pub last_successful_at: Option<Timestamp>,
    pub degraded_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotDegraded;

impl std::fmt::Display for NotDegraded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a degraded source needs a reason and stale freshness or incomplete quality")
    }
}

impl std::error::Error for NotDegraded {}

impl SourceHealth {
    pub fn healthy(source_id: impl Into<String>, observed_at: Timestamp) -> Self {
        Self {
            source_id: source_id.into(),
            freshness: SourceFreshness::Fresh,
            quality: SourceQuality::Complete,
            last_successful_at: Some(observed_at),
            degraded_reason: None,
        }
    }

    pub fn degraded(
        source_id: impl Into<String>,
        freshness: SourceFreshness,
        quality: SourceQuality,
        reason: impl Into<String>,
        last_successful_at: Option<Timestamp>,
    ) -> Result<Self, NotDegraded> {
        let reason = reason.into();
        if reason.trim().is_empty()
            || (freshness == SourceFreshness::Fresh && quality == SourceQuality::Complete)
        {
            return Err(NotDegraded);
        }
        Ok(Self {
            source_id: source_id.into(),
            freshness,
            quality,
            last_successful_at,
            degraded_reason: Some(reason),
        })
    }

    pub fn is_degraded(&self) -> bool {
        self.freshness != SourceFreshness::Fresh || self.quality != SourceQuality::Complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_source_captures_successful_observation() {
        let observed_at = Timestamp::from_second(1_785_715_200).unwrap();

        let health = SourceHealth::healthy("activitywatch", observed_at);

        assert_eq!(health.freshness, SourceFreshness::Fresh);
        assert_eq!(health.quality, SourceQuality::Complete);
        assert_eq!(health.last_successful_at, Some(observed_at));
    }

    #[test]
    fn degraded_factory_preserves_operational_diagnostics() {
        let health = SourceHealth::degraded(
            "activitywatch",
            SourceFreshness::Stale,
            SourceQuality::Incomplete,
            "history-incomplete",
            None,
        )
        .unwrap();

        assert_eq!(health.freshness, SourceFreshness::Stale);
        assert_eq!(health.quality, SourceQuality::Incomplete);
        assert_eq!(
            health.degraded_reason.as_deref(),
            Some("history-incomplete")
        );
    }

    #[test]
    fn degraded_factory_rejects_healthy_state_and_blank_reason() {
        let healthy = SourceHealth::degraded(
            "activitywatch",
            SourceFreshness::Fresh,
            SourceQuality::Complete,
            "not-actually-degraded",
            None,
        );
        let blank = SourceHealth::degraded(
            "activitywatch",
            SourceFreshness::Stale,
            SourceQuality::Complete,
            " ",
            None,
        );

        assert_eq!(healthy, Err(NotDegraded));
        assert_eq!(blank, Err(NotDegraded));
    }
}
