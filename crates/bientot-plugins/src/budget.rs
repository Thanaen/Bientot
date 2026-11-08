//! CPU budgets (FR-014): a token bucket per plugin and one for all plugins together. Time spent in
//! a plugin, host work for it included, is taken from both; an empty bucket delays that plugin's
//! background events, never a user action.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bientot_plugin_api::PluginId;

pub const PLUGIN_RATE: Duration = Duration::from_millis(20);
pub const PLUGIN_RESERVE: Duration = Duration::from_millis(500);
pub const GLOBAL_RATE: Duration = Duration::from_millis(60);
pub const GLOBAL_RESERVE: Duration = Duration::from_secs(1);
const MINUTE: Duration = Duration::from_secs(60);

/// Refills by `rate` per minute up to `reserve`; may go below zero, which is a debt the refill
/// pays first.
#[derive(Debug, Clone)]
pub struct Bucket {
    rate: Duration,
    reserve: Duration,
    /// Microseconds; negative while in debt.
    level: i64,
    at: Instant,
}

impl Bucket {
    pub fn new(rate: Duration, reserve: Duration, now: Instant) -> Bucket {
        Bucket {
            rate,
            reserve,
            level: micros(reserve),
            at: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.at);
        let earned =
            micros(self.rate) as i128 * elapsed.as_micros() as i128 / MINUTE.as_micros() as i128;
        self.level = (self.level as i128 + earned).min(micros(self.reserve) as i128) as i64;
        self.at = now;
    }

    pub fn charge(&mut self, used: Duration, now: Instant) {
        self.refill(now);
        self.level = self.level.saturating_sub(micros(used));
    }

    /// When the bucket holds something again: `now` if it already does.
    pub fn ready_at(&mut self, now: Instant) -> Instant {
        self.refill(now);
        if self.level > 0 {
            return now;
        }
        let missing = (1 - self.level) as u128;
        let wait = missing * MINUTE.as_micros() / self.rate.as_micros().max(1);
        now + Duration::from_micros(wait.min(u64::MAX as u128) as u64)
    }
}

fn micros(duration: Duration) -> i64 {
    duration.as_micros().min(i64::MAX as u128) as i64
}

pub struct Budgets {
    global: Bucket,
    plugins: HashMap<PluginId, Bucket>,
}

impl Budgets {
    pub fn new(now: Instant) -> Budgets {
        Budgets {
            global: Bucket::new(GLOBAL_RATE, GLOBAL_RESERVE, now),
            plugins: HashMap::new(),
        }
    }

    fn plugin(&mut self, id: &PluginId, now: Instant) -> &mut Bucket {
        self.plugins
            .entry(id.clone())
            .or_insert_with(|| Bucket::new(PLUGIN_RATE, PLUGIN_RESERVE, now))
    }

    /// When a background event for `id` may run.
    pub fn background_ready_at(&mut self, id: &PluginId, now: Instant) -> Instant {
        let global = self.global.ready_at(now);
        self.plugin(id, now).ready_at(now).max(global)
    }

    pub fn charge(&mut self, id: &PluginId, used: Duration, now: Instant) {
        self.global.charge(used, now);
        self.plugin(id, now).charge(used, now);
    }

    pub fn forget(&mut self, id: &PluginId) {
        self.plugins.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> PluginId {
        PluginId::parse(name).unwrap()
    }

    #[test]
    fn the_reserve_then_the_rate() {
        let t0 = Instant::now();
        let mut bucket = Bucket::new(PLUGIN_RATE, PLUGIN_RESERVE, t0);
        assert_eq!(bucket.ready_at(t0), t0);
        bucket.charge(Duration::from_millis(500), t0);
        // Empty: 1 µs of credit comes after 3 ms at 20 ms per minute.
        assert_eq!(bucket.ready_at(t0), t0 + Duration::from_millis(3));
        bucket.charge(Duration::from_millis(100), t0);
        // 100 ms of debt is paid in 5 minutes.
        let ready = bucket.ready_at(t0);
        assert!(ready > t0 + Duration::from_secs(300) && ready < t0 + Duration::from_secs(301));
        // Refills stop at the reserve.
        let later = t0 + Duration::from_secs(3600);
        bucket.charge(Duration::ZERO, later);
        assert_eq!(bucket.level, 500_000);
    }

    #[test]
    fn one_plugin_does_not_starve_another_until_the_global_bucket_is_empty() {
        let t0 = Instant::now();
        let mut budgets = Budgets::new(t0);
        let (a, b) = (id("bientot.a"), id("bientot.b"));
        budgets.charge(&a, Duration::from_millis(600), t0);
        assert!(budgets.background_ready_at(&a, t0) > t0);
        assert_eq!(budgets.background_ready_at(&b, t0), t0);
        budgets.charge(&b, Duration::from_millis(450), t0);
        // The global 1 s reserve is spent: b waits too.
        assert!(budgets.background_ready_at(&b, t0) > t0);
        let after = t0 + Duration::from_secs(60);
        assert_eq!(budgets.background_ready_at(&b, after), after);
    }
}
