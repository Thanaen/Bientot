//! « Tomato »: a 25-minute focus timer with 5-minute breaks, run from the card or the palette.
//!
//! It needs only `ui:flyout`: it never reads the day. Bientôt draws the countdown, so the plugin
//! sleeps between two presses. An instance can be dropped after five idle minutes, so the card's
//! buttons carry the timer's state in their `arg` (`<tomatoes done>` or
//! `<tomatoes done>:<end of the focus>`), and a new instance picks it up from the next press.

use bientot_plugin_sdk::prelude::*;

const FOCUS: SignedDuration = SignedDuration::from_mins(25);
const BREAK: SignedDuration = SignedDuration::from_mins(5);
/// Past this many, the count is written in figures.
const SHOWN: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Phase {
    #[default]
    Idle,
    Focus {
        until: Timestamp,
    },
    Break {
        until: Timestamp,
    },
}

#[derive(Default)]
struct Tomato {
    phase: Phase,
    done: u32,
}

impl Tomato {
    /// What the press's `arg` says, which wins over memory: this instance may be new.
    fn restore(&mut self, arg: Option<&str>) {
        let Some(arg) = arg else { return };
        let (done, until) = match arg.split_once(':') {
            Some((done, until)) => (done, until.parse::<i64>().ok()),
            None => (arg, None),
        };
        if let Ok(done) = done.parse() {
            self.done = done;
        }
        if let Some(until) = until.and_then(|s| Timestamp::from_second(s).ok()) {
            self.phase = Phase::Focus { until };
        }
    }

    /// A focus that ran to its end counts, whatever was pressed after it.
    fn settle(&mut self, now: Timestamp) {
        if let Phase::Focus { until } = self.phase
            && until <= now
        {
            self.done += 1;
            self.phase = Phase::Idle;
        }
    }

    fn arg(&self) -> String {
        match self.phase {
            Phase::Focus { until } => format!("{}:{}", self.done, until.as_second()),
            _ => self.done.to_string(),
        }
    }

    fn show(&self) {
        let arg = self.arg();
        let tally = match self.done {
            0 => None,
            n if n > SHOWN => Some(text("today-many").arg("count", n).muted()),
            n => Some(
                text("today")
                    .arg("tomatoes", "🍅".repeat(n as usize))
                    .muted(),
            ),
        };
        let card = match self.phase {
            Phase::Idle => card()
                .line(text("ready"))
                .action(button("start", text("start")).arg(&arg)),
            Phase::Focus { until } => card()
                .line(row![text("focus"), countdown(until)])
                .action(button("break", text("short-break")).arg(&arg))
                .action(button("stop", text("stop")).arg(&arg)),
            Phase::Break { until } => card()
                .line(row![text("break"), countdown(until)])
                .action(button("start", text("start")).arg(&arg))
                .action(button("stop", text("stop")).arg(&arg)),
        };
        let _ = ui::show(card.line_if(tally));
    }
}

impl Plugin for Tomato {
    fn activate(&mut self, _: Language, reason: ActivateReason) {
        // After an eviction the card on screen is still right: keep it.
        if matches!(reason, ActivateReason::First | ActivateReason::Restart) {
            self.show();
        }
    }

    fn command(&mut self, command: &str, arg: Option<&str>, _: CommandSource) {
        let now = time::now();
        self.restore(arg);
        self.settle(now);
        self.phase = match command {
            "start" => Phase::Focus { until: now + FOCUS },
            "break" => Phase::Break { until: now + BREAK },
            _ => Phase::Idle,
        };
        self.show();
    }
}

bientot_plugin_sdk::export!(Tomato);

#[cfg(test)]
mod tests {
    use super::*;
    use bientot_plugin_sdk::testing::{TestHost, assert_reference, demo_report};

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/tomato");

    #[test]
    fn every_demo_day() {
        for language in [Language::Fr, Language::En] {
            let report = demo_report::<Tomato>(DIR, language);
            assert_reference(format!("{DIR}/expected/{}.txt", language.tag()), &report);
        }
    }

    #[test]
    fn a_new_instance_picks_up_the_timer_from_the_button() {
        let mut host = TestHost::load(DIR);
        host.at("2026-11-09T09:00:00Z".parse().unwrap());
        let mut plugin = Tomato::default();
        host.start(&mut plugin).unwrap();
        host.press(&mut plugin, 0).unwrap();
        // Five idle minutes: Bientôt drops the instance and starts a fresh one.
        let mut plugin = Tomato::default();
        host.send(
            &mut plugin,
            Event::Activate {
                language: Language::En,
                reason: ActivateReason::Evicted,
            },
        )
        .unwrap();
        assert!(
            host.render().contains("Focus, time left ⏳0:25"),
            "{}",
            host.render()
        );
        host.advance(SignedDuration::from_mins(26));
        host.press(&mut plugin, 0).unwrap();
        assert!(host.render().contains("Today: 🍅"), "{}", host.render());
    }

    #[test]
    fn a_focus_cut_short_does_not_count() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Tomato::default();
        host.start(&mut plugin).unwrap();
        host.command(&mut plugin, "start", None).unwrap();
        host.advance(SignedDuration::from_mins(10));
        host.command(&mut plugin, "break", None).unwrap();
        assert!(!host.render().contains('🍅'));
    }
}
