//! The text of the reminders `worktime-core::notifications` decides to show, as
//! `Presentation/Notifications/NotificationTextProvider.cs` writes it, and on Windows the sink
//! that shows them as toasts.

use worktime_core::notifications::{Notification, NotificationBody};
use worktime_domain::time::TimeZoneProvider;
use worktime_windows::notifications::NotificationRequest;

use crate::strings::{Key, Language};

/// The toast of `notification`. The projected departure is a time of the workday's zone, and
/// empty when that zone is unknown here.
pub fn toast(
    notification: &Notification,
    language: Language,
    zones: &dyn TimeZoneProvider,
) -> NotificationRequest {
    let key = match notification.body() {
        NotificationBody::CompletionApproach => Key::NotificationCompletionBody,
        NotificationBody::LatestDepartureApproach => Key::NotificationDepartureBody,
        NotificationBody::Combined => Key::NotificationCombinedBody,
        NotificationBody::Complete => Key::NotificationCompleteBody,
    };
    let projected = notification
        .projected_departure
        .zip(zones.get(&notification.time_zone_id).ok())
        .map(|(departure, zone)| language.short_time(departure.to_zoned(zone).time()))
        .unwrap_or_default();
    let mut body = language.format(key, &[&notification.remaining_minutes(), &projected]);
    if notification.is_estimated {
        body = format!(
            "{body} {}",
            language.text(Key::NotificationEstimateQualifier)
        );
    }
    NotificationRequest {
        delivery_id: notification.delivery_id.clone(),
        title: language.text(Key::NotificationTitle).to_owned(),
        body,
    }
}

/// Shows the reminders as toasts in the language of the launch.
#[cfg(windows)]
pub struct ToastSink {
    pub toasts: worktime_windows::notifications::Toasts,
    pub language: Language,
    pub zones: &'static dyn TimeZoneProvider,
}

#[cfg(windows)]
impl worktime_core::notifications::NotificationSink for ToastSink {
    fn show(
        &self,
        notification: &Notification,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(self
            .toasts
            .show(&toast(notification, self.language, self.zones))?)
    }
}

#[cfg(test)]
mod tests;
