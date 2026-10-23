//! « Corriger le temps… » from the week: on the open day, on a correction to replace, or as a
//! declaration of work on the open day.

use gpui::{AnyElement, Context, Window};
use worktime_domain::accounting::AdjustmentId;
use worktime_domain::time::LocalWorkday;

use super::{ActionButton, Click, WeekView};
use crate::adjust::Seed;
use crate::adjust::window::{OpenOutcome, Opener};
use crate::adjust_view;
use crate::flyout::AdjustTime;
use crate::host::Today;
use crate::strings::Key;
use crate::theme::Theme;
use crate::week::WeekState;

impl WeekView {
    /// « Corriger le temps… » and Ctrl+E, on the open day.
    pub(super) fn adjust_time(&mut self, _: &AdjustTime, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(workday) = self.state.as_ref().and_then(WeekState::adjust_target) {
            let _ = adjust_view::window::open(Some(workday), Seed::Blank, Opener::Week, cx);
        }
    }

    /// « Renseigner un créneau », on the open day.
    fn declare_work(&mut self, cx: &mut Context<Self>) {
        if let Some((workday, seed)) = self.state.as_ref().and_then(WeekState::declare_work) {
            let _ = adjust_view::window::open(Some(workday), seed, Opener::Week, cx);
        }
    }

    /// « Corriger le temps… » on the correction `id` of `workday`, as the ledger holds it now,
    /// unless another replaced it since the week was read.
    pub(super) fn open_edit(
        &mut self,
        workday: LocalWorkday,
        id: AdjustmentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> OpenOutcome {
        let Some(state) = &mut self.state else {
            return OpenOutcome::Refused;
        };
        window.focus(&self.focus_handle, cx);
        let edited = if cx.has_global::<Today>() {
            cx.global_mut::<Today>().editable(&workday, &id)
        } else {
            Err(Key::HistoryCorrectionFailed)
        };
        let opened = match edited {
            Ok(edited) => {
                adjust_view::window::open(Some(workday), Seed::Edit(edited), Opener::Week, cx)
            }
            Err(message) => {
                state.edit_refused(message);
                OpenOutcome::Refused
            }
        };
        cx.notify();
        opened
    }

    pub(super) fn declare_button(
        &self,
        enabled: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.action_button(
            ActionButton {
                id: "declare-work",
                label: Key::HistoryDeclareWorkContent,
                icon: None,
                shortcut: None,
                click: Click::enabled_if(
                    enabled,
                    cx.listener(|view, _, _, cx| view.declare_work(cx)),
                ),
            },
            theme,
        )
    }
}
