//! The text of the reminders `bientot-core::notifications` decides to show, as
//! `Presentation/Notifications/NotificationTextProvider.cs` writes it, and the sink that has the
//! system show them.

use bientot_core::notifications::{Notification, NotificationBody, NotificationSink};
use bientot_domain::time::TimeZoneProvider;
use bientot_platform::notifications::{NotificationRequest, Notifier};

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
        .map(|(departure, zone)| language.short_time(departure.to_zoned_datetime(zone).time()))
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

/// Shows the reminders through the system's notifications, in the language of the launch.
pub struct ToastSink {
    pub notifier: Box<dyn Notifier>,
    pub language: Language,
    pub zones: &'static dyn TimeZoneProvider,
}

impl NotificationSink for ToastSink {
    fn show(
        &self,
        notification: &Notification,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.notifier
            .show(&toast(notification, self.language, self.zones))
    }
}

#[cfg(test)]
mod tests;
