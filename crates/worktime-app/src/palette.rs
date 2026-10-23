//! The command palette Ctrl+Maj+P opens over the flyout and the week: which commands run where,
//! their texts, and the query that filters and ranks them. `palette_view` only draws it.

use std::ops::Range;

use gpui::SharedString;
use worktime_domain::accounting::TrackingState;

use crate::flyout::{self, keystroke};
use crate::live::Tracking;
use crate::strings::{Key, Language};
use crate::week_view::keystroke as week;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Pauses a running day, resumes a paused or finished one.
    TogglePause,
    AdjustTime,
    PreviousDay,
    NextDay,
    SettleNext,
    Undo,
    FinishDay,
    OpenWeek,
    OpenSettings,
    OpenSettingsJson,
    Refresh,
    ExportDiagnostics,
    Quit,
}

impl Command {
    /// Every command, in the order the palette lists them.
    pub const ALL: [Command; 13] = [
        Command::TogglePause,
        Command::AdjustTime,
        Command::PreviousDay,
        Command::NextDay,
        Command::SettleNext,
        Command::Undo,
        Command::FinishDay,
        Command::OpenWeek,
        Command::OpenSettings,
        Command::OpenSettingsJson,
        Command::Refresh,
        Command::ExportDiagnostics,
        Command::Quit,
    ];

    /// The keystroke that runs it outside the palette, the one its window binds.
    pub fn keystroke(self) -> Option<&'static str> {
        match self {
            Command::TogglePause => Some(keystroke::TOGGLE_PAUSE),
            Command::AdjustTime => Some(keystroke::ADJUST_TIME),
            Command::PreviousDay => Some(week::PREVIOUS_DAY),
            Command::NextDay => Some(week::NEXT_DAY),
            Command::SettleNext => Some(week::SETTLE_NEXT),
            Command::Undo => Some(week::UNDO),
            Command::FinishDay => Some(keystroke::FINISH_DAY),
            Command::OpenWeek => Some(keystroke::OPEN_WEEK),
            Command::OpenSettings => Some(keystroke::OPEN_SETTINGS),
            Command::Refresh => Some(keystroke::REFRESH),
            Command::Quit => Some(keystroke::QUIT),
            Command::OpenSettingsJson | Command::ExportDiagnostics => None,
        }
    }

    fn domain(self) -> Key {
        match self {
            Command::TogglePause => Key::PaletteDomainTracking,
            Command::AdjustTime => Key::PaletteDomainTime,
            Command::PreviousDay
            | Command::NextDay
            | Command::SettleNext
            | Command::Undo
            | Command::OpenWeek => Key::PaletteDomainWeek,
            Command::FinishDay => Key::PaletteDomainDay,
            Command::OpenSettings | Command::OpenSettingsJson => Key::PaletteDomainSettings,
            Command::Refresh | Command::ExportDiagnostics | Command::Quit => Key::PaletteDomainApp,
        }
    }

    fn verb(self, tracking: TrackingState) -> Key {
        match self {
            Command::TogglePause if tracking == TrackingState::Running => Key::PalettePause,
            Command::TogglePause => Key::PaletteResume,
            Command::AdjustTime => Key::PaletteCorrect,
            Command::PreviousDay => Key::PalettePreviousDay,
            Command::NextDay => Key::PaletteNextDay,
            Command::SettleNext => Key::PaletteSettleNext,
            Command::Undo => Key::PaletteUndo,
            Command::FinishDay => Key::PaletteFinish,
            Command::OpenWeek | Command::OpenSettings => Key::PaletteOpen,
            Command::OpenSettingsJson => Key::PaletteOpenSettingsJson,
            Command::Refresh => Key::PaletteRefresh,
            Command::ExportDiagnostics => Key::PaletteExportDiagnostics,
            Command::Quit => Key::PaletteQuit,
        }
    }

    fn runs_in(self, situation: &Situation) -> bool {
        let week = situation.host == Host::Week;
        match self {
            Command::PreviousDay | Command::NextDay | Command::SettleNext | Command::Undo => week,
            Command::OpenWeek => !week,
            Command::AdjustTime => situation.can_correct_time,
            // As the flyout and the tray menu grey them.
            Command::TogglePause => situation.tracking.can_toggle(),
            Command::FinishDay => situation.tracking.can_finish(),
            _ => true,
        }
    }
}

/// The window the palette opens over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Flyout,
    Week,
}

/// What decides the commands the palette offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Situation {
    pub host: Host,
    pub tracking: Tracking,
    /// « Corriger le temps… » has a working day to correct.
    pub can_correct_time: bool,
}

/// One command as the palette lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub command: Command,
    /// « Suivi : mettre en pause ».
    pub label: String,
    /// Where the verb starts in `label`; the domain before it is muted.
    pub verb_start: usize,
    /// The part of `label` the query matched.
    pub hit: Option<Range<usize>>,
    /// « Suivi : mettre en pause, Ctrl+Alt+P », as a screen reader reads the row.
    pub name: SharedString,
}

pub struct Palette {
    situation: Situation,
    language: Language,
    query: String,
    selected: usize,
}

impl Palette {
    pub fn new(situation: Situation, language: Language) -> Self {
        Self {
            situation,
            language,
            query: String::new(),
            selected: 0,
        }
    }

    /// The day changed while the palette is open: « Suivi : mettre en pause » may now resume.
    pub fn set_situation(&mut self, situation: Situation) {
        self.situation = situation;
        self.clamp();
    }

    /// A new query selects its first row.
    pub fn set_query(&mut self, query: &str) {
        if self.query != query {
            query.clone_into(&mut self.query);
            self.selected = 0;
        }
    }

    /// The heading above the rows: « Commandes » until a query filters them.
    pub fn heading(&self) -> Option<&'static str> {
        self.folded_query()
            .is_empty()
            .then(|| self.language.text(Key::PaletteCommands))
    }

    /// The commands that run here, matching the query: those it matches at the start of their
    /// domain or verb first, each group in the palette's order.
    pub fn rows(&self) -> Vec<Row> {
        let query = self.folded_query();
        let mut ranked: Vec<(bool, Row)> = Command::ALL
            .into_iter()
            .filter(|command| command.runs_in(&self.situation))
            .filter_map(|command| {
                let mut row = self.row(command);
                if query.is_empty() {
                    return Some((true, row));
                }
                let (hit, at_start) = find(&row.label, row.verb_start, &query)?;
                row.hit = Some(hit);
                Some((at_start, row))
            })
            .collect();
        ranked.sort_by_key(|(at_start, _)| !at_start);
        ranked.into_iter().map(|(_, row)| row).collect()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select_next(&mut self) {
        self.selected += 1;
        self.clamp();
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// The command Enter runs, if any row is left.
    pub fn chosen(&self) -> Option<Command> {
        self.rows().get(self.selected).map(|row| row.command)
    }

    fn clamp(&mut self) {
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
    }

    fn folded_query(&self) -> Vec<char> {
        self.query.trim().chars().map(fold).collect()
    }

    fn row(&self, command: Command) -> Row {
        let language = self.language;
        let (domain, verb) = (
            language.text(command.domain()),
            language.text(command.verb(self.situation.tracking.state())),
        );
        let label = language.format(Key::PaletteCommandFormat, &[&domain, &verb]);
        let verb_start = label.len() - verb.len();
        let name = match command.keystroke() {
            Some(keystroke) => flyout::shortcut_name(&label, keystroke, language),
            None => label.clone().into(),
        };
        Row {
            command,
            label,
            verb_start,
            hit: None,
            name,
        }
    }
}

/// Where `query`, folded, first occurs in `label`: at the start of the domain or of the verb if
/// it occurs there, the second value then true.
fn find(label: &str, verb_start: usize, query: &[char]) -> Option<(Range<usize>, bool)> {
    let chars: Vec<(usize, char)> = label.char_indices().map(|(at, c)| (at, fold(c))).collect();
    let starts: Vec<usize> = (0..=chars.len().saturating_sub(query.len()))
        .filter(|&start| {
            chars[start..]
                .iter()
                .map(|(_, c)| c)
                .take(query.len())
                .eq(query)
        })
        .collect();
    let byte = |index: usize| chars.get(index).map_or(label.len(), |(at, _)| *at);
    let preferred = starts
        .iter()
        .copied()
        .find(|&start| byte(start) == 0 || byte(start) == verb_start);
    let start = preferred.or_else(|| starts.first().copied())?;
    Some((byte(start)..byte(start + query.len()), preferred.is_some()))
}

/// `c` as the query compares it: lowercase, without its accent, a no-break space as a space.
fn fold(c: char) -> char {
    match c.to_lowercase().next().unwrap_or(c) {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        '\u{a0}' | '\u{202f}' => ' ',
        lower => lower,
    }
}

#[cfg(test)]
mod tests;
