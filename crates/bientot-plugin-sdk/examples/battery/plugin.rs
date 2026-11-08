//! « Battery »: the day as a battery that runs down from the first minute of work to the target,
//! with a mood for each level and a coffee counter. The counter rides in the button's `arg`, so
//! that an instance started after an eviction keeps it.

use bientot_plugin_sdk::prelude::*;

#[derive(Default)]
struct Battery {
    coffees: u32,
}

/// The mood of a charge in percent, and its tone.
fn mood(percent: i64) -> (&'static str, Tone) {
    match percent {
        76.. => ("full", Tone::Success),
        41..=75 => ("cruising", Tone::Info),
        11..=40 => ("low", Tone::Warning),
        1..=10 => ("saver", Tone::Warning),
        _ => ("flat", Tone::Muted),
    }
}

impl Battery {
    fn show(&self, day: &DaySnapshot) {
        let coffees = (self.coffees > 0).then(|| {
            text("coffees")
                .arg("cups", "☕".repeat(self.coffees.min(12) as usize))
                .muted()
        });
        let coffee = button("coffee", text("coffee")).arg(self.coffees);
        if day.target <= 0 || day.tracking == Tracking::NonWorkingDay {
            let _ = ui::show(
                card()
                    .line(badge(text("rest"), Tone::Info))
                    .line_if(coffees),
            );
            return;
        }
        let left = (day.target - day.counted).clamp(0, day.target);
        let percent = (left * 100 + day.target / 2) / day.target;
        let (mood, tone) = mood(percent);
        let recharge = day
            .departure_at_target
            .map(|at| row![text("recharge").muted(), time_at(at)]);
        let _ = ui::show(
            card()
                .line(progress(
                    left as f64,
                    day.target as f64,
                    text("charge").arg("percent", percent),
                ))
                .line(badge(text(mood), tone))
                .line_if(recharge)
                .line_if(coffees)
                .action(coffee),
        );
    }
}

impl Plugin for Battery {
    fn activate(&mut self, _: Language, _: ActivateReason) {
        if let Some(day) = day::current() {
            self.show(&day);
        }
    }

    fn day_refreshed(&mut self, day: &DaySnapshot) {
        self.show(day);
    }

    fn command(&mut self, command: &str, arg: Option<&str>, _: CommandSource) {
        if command != "coffee" {
            return;
        }
        let before = arg.and_then(|a| a.parse().ok()).unwrap_or(self.coffees);
        self.coffees = before + 1;
        log::info(&format!("coffee number {}", self.coffees));
        if let Some(day) = day::current() {
            self.show(&day);
        }
    }

    fn language_changed(&mut self, language: Language) {
        log::info(&format!("the app now speaks {}", language.tag()));
    }
}

bientot_plugin_sdk::export!(Battery);

#[cfg(test)]
mod tests {
    use super::*;
    use bientot_plugin_sdk::testing::{TestHost, assert_reference, demo_day, demo_report};

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/battery");

    #[test]
    fn every_demo_day() {
        for language in [Language::Fr, Language::En] {
            let report = demo_report::<Battery>(DIR, language);
            assert_reference(format!("{DIR}/expected/{}.txt", language.tag()), &report);
        }
    }

    #[test]
    fn the_battery_runs_down_with_the_day() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Battery::default();
        host.refresh(&mut plugin, demo_day("Morning")).unwrap();
        assert!(host.render().contains("Battery 71%"), "{}", host.render());
        host.refresh(&mut plugin, demo_day("Complete")).unwrap();
        assert!(host.render().contains("Battery 0%"), "{}", host.render());
        assert!(host.render().contains("recharge tonight"));
    }

    #[test]
    fn coffees_survive_a_new_instance_and_the_language_is_logged() {
        let mut host = TestHost::load(DIR);
        let mut plugin = Battery::default();
        host.refresh(&mut plugin, demo_day("OnTrack")).unwrap();
        host.press(&mut plugin, 0).unwrap();
        let mut fresh = Battery::default();
        host.press(&mut fresh, 0).unwrap();
        assert!(host.render().contains("Coffees: ☕☕"), "{}", host.render());
        host.change_language(&mut fresh, Language::Fr).unwrap();
        assert!(host.render().contains("Cafés\u{a0}: ☕☕"));
        let logs = host.logs();
        assert_eq!(logs.last().unwrap().text, "the app now speaks fr");
    }
}
