//! What the app asks the system to show as a notification, and the clicks on what it showed.

use std::sync::{Arc, Mutex, PoisonError};

/// One toast: two lines of text, and the delivery id the receipts recorded it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationRequest {
    pub delivery_id: String,
    pub title: String,
    pub body: String,
}

/// Called with the delivery id of a clicked notification, when the system gives it.
pub type ClickHandler = Box<dyn Fn(Option<&str>) + Send + Sync>;

/// The clicks on the body of the notifications this process showed.
pub trait NotificationClicks {
    /// Hands `handler` each click from now on, after those that arrived before, in order.
    fn on_click(&self, handler: ClickHandler);
}

/// Clicks held until a handler takes them. A clone shares the clicks: the system's callback
/// [`Self::click`]s one, and the app hands its handler to another.
#[derive(Clone, Default)]
pub struct QueuedClicks(Arc<Mutex<Clicks>>);

enum Clicks {
    Waiting(Vec<Option<String>>),
    Handled(ClickHandler),
}

impl Default for Clicks {
    fn default() -> Self {
        Self::Waiting(Vec::new())
    }
}

impl QueuedClicks {
    pub fn click(&self, delivery_id: Option<String>) {
        match &mut *self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Clicks::Waiting(waiting) => waiting.push(delivery_id),
            Clicks::Handled(handler) => handler(delivery_id.as_deref()),
        }
    }
}

impl NotificationClicks for QueuedClicks {
    fn on_click(&self, handler: ClickHandler) {
        let mut clicks = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Clicks::Waiting(waiting) = &*clicks {
            for delivery_id in waiting {
                handler(delivery_id.as_deref());
            }
        }
        *clicks = Clicks::Handled(handler);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn clicks_before_the_handler_reach_it_first_and_in_order() {
        let clicks = QueuedClicks::default();
        let (sender, received) = mpsc::channel();
        clicks.click(Some("first".into()));
        clicks.click(None);

        clicks.clone().on_click(Box::new(move |delivery_id| {
            sender.send(delivery_id.map(str::to_owned)).unwrap();
        }));
        clicks.click(Some("later".into()));

        assert_eq!(
            received.try_iter().collect::<Vec<_>>(),
            vec![Some("first".into()), None, Some("later".into())]
        );
    }
}
