//! The flyout's body, between the header and the actions: the notices, the day, the
//! announcements and the Store update on offer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Bounds, Context, ElementId, FocusHandle, Pixels,
    Role, ScrollHandle, SharedString, Transformation, Window, div, percentage, prelude::*, px,
};

use super::{BODY_PADDING, Flyout, InstallUpdate, readable};
use crate::settings_view;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size, tabular_figures, text};
use crate::updates::{UpdateCallout, Updates};
use crate::widgets::{Hue, Notice, NoticeKind, callout_box, icon, notice_box, small};
use crate::workday::Worked;

/// Where a notice that arrives in the open flyout stands at `time`, a share of its entrance: it
/// sits in the flow, so it fades in over the whole entrance while it rises 4 px into place.
fn notice_pose(time: f32) -> motion::Entering {
    motion::slide_in(time, px(4.))
}

/// What the flyout tells without being asked, below the answer to an action.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Alerts {
    /// What went wrong while WorkTime started, told in the first flyout for as long as it stays
    /// open.
    pub announcements: Vec<Notice>,
    /// The last refresh failed: told until one succeeds.
    pub refresh_failure: Option<Notice>,
}

/// The refresh failure the flyout shows under `answer`: none while the answer already asks for
/// a refresh.
fn shown_refresh_failure<'a>(
    answer: Option<&Notice>,
    failure: Option<&'a Notice>,
) -> Option<&'a Notice> {
    failure.filter(|_| answer.is_none_or(|answer| answer.kind != NoticeKind::RefreshPending))
}

/// What a part of the body shows, so that the focus finds the part after a callout that closed
/// whatever came or went around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Notice,
    RefreshFailure,
    SourceCallout,
    AppointmentCallout,
    CountedCallout,
    TrustCallout,
    Headline,
    Worked,
    Announcement(usize),
    UpdateCallout,
}

impl Flyout {
    /// `notice` at `time` of its entrance. Only one notice shown asks for a refresh, see
    /// [`shown_refresh_failure`], and only one tells of restored settings, so their buttons keep
    /// one id.
    fn notice(
        &self,
        id: impl Into<ElementId>,
        notice: &Notice,
        time: f32,
        theme: &Theme,
    ) -> AnyElement {
        let action = match notice.kind {
            NoticeKind::RefreshPending => Some(self.refresh_button("notice-refresh", theme)),
            NoticeKind::SettingsRecovered => {
                Some(self.open_settings_button("notice-open-settings", theme))
            }
            NoticeKind::Success
            | NoticeKind::Warning
            | NoticeKind::SourceSwitchFailed
            | NoticeKind::Error => None,
        };
        notice_box(id, notice, theme)
            .map(|notice| notice_pose(time).apply(notice))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(BASE_02)
                    .child(notice.message.clone())
                    .children(action.map(|action| div().pt(BASE_06).flex().child(action))),
            )
            .into_any_element()
    }

    /// The answer to an action, then the failed refresh: what bears on the day's numbers, above
    /// them.
    fn notices(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        theme: &Theme,
    ) -> Vec<(Part, AnyElement)> {
        let answer_time = match self.notice {
            Some(_) => self.notice_entrance.time(motion::MENU, window, cx),
            None => 1.,
        };
        let failure =
            shown_refresh_failure(self.notice.as_ref(), self.alerts.refresh_failure.as_ref())
                .cloned();
        let failure_time = match failure {
            Some(_) => self.alert_entrance.time(motion::MENU, window, cx),
            None => 1.,
        };
        let mut notices = Vec::new();
        if let Some(answer) = &self.notice {
            notices.push((
                Part::Notice,
                self.notice("notice", answer, answer_time, theme),
            ));
        }
        if let Some(failure) = &failure {
            notices.push((
                Part::RefreshFailure,
                self.notice("refresh-failure", failure, failure_time, theme),
            ));
        }
        notices
    }

    /// What went wrong while WorkTime started, below the day so that the planned departure stays
    /// on top.
    fn announcements(&self, theme: &Theme) -> Vec<(Part, AnyElement)> {
        self.alerts
            .announcements
            .iter()
            .enumerate()
            .map(|(index, announcement)| {
                (
                    Part::Announcement(index),
                    self.notice(("announcement", index), announcement, 1., theme),
                )
            })
            .collect()
    }

    /// The update on offer, below the day: the lowest of the flyout's callouts.
    fn update_callout(&self, theme: &Theme, cx: &App) -> Option<AnyElement> {
        let updates = cx.try_global::<Updates>()?;
        let callout = updates.callout()?;
        let (message, label) = callout.text(updates.restarts());
        let role = match callout {
            UpdateCallout::Failed => Role::Alert,
            UpdateCallout::Offered | UpdateCallout::Installing => Role::Status,
        };
        let (message, label) = (self.text(message), self.text(label));
        let id = "install-update";
        let button = if callout == UpdateCallout::Installing {
            let spinner = icon("loader-circle", theme.text_muted)
                .size(size::ICON_SM)
                .with_animation(
                    "installing",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |icon, delta| {
                        icon.with_transformation(Transformation::rotate(percentage(delta)))
                    },
                );
            // Disabled: out of the Tab order and deaf to clicks until the Store answers. The
            // same id keeps the focus of « Mettre à jour » pressed with the keyboard.
            let focused = theme.border_focused;
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                // GPUI has no setter for these states, only this hook on the node.
                .a11y_synthetic_children(|children| {
                    let node = children.parent_node();
                    node.set_disabled();
                    node.set_busy();
                })
                .tab_index(0)
                .tab_stop(false)
                .focus_visible(move |style| style.border_color(focused))
                .h(size::BUTTON_MEDIUM)
                .px(BASE_12)
                .flex()
                .items_center()
                .gap(BASE_06)
                .rounded(radius::SM)
                .border_1()
                .border_color(theme.border_on_tint)
                .text_color(theme.text_placeholder)
                .whitespace_nowrap()
                .child(spinner)
                .child(label)
        } else {
            settings_view::outlined_on_tint(id, label, size::BUTTON_MEDIUM, theme)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(InstallUpdate), cx))
        };
        Some(
            callout_box(Hue::Info, theme)
                .id("update")
                .role(role)
                .aria_label(message)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(BASE_02)
                        .child(message)
                        .child(
                            div()
                                .pt(BASE_06)
                                .flex()
                                .child(self.reveal_on_tab(id, button)),
                        ),
                )
                .into_any_element(),
        )
    }

    fn headline(&self, theme: &Theme) -> impl IntoElement {
        let headline = self.day.shown_while(self.tracking.state()).headline;
        let value = if headline.emphasized {
            div().type_style(text::WT_HERO)
        } else {
            div()
                .type_style(text::LABEL_LG)
                .text_color(theme.text_muted)
        };
        let name = match &headline.label {
            Some(label) => self
                .language
                .format(Key::WorkdayPrimaryValueFormat, &[label, &headline.value]),
            None => headline.value.to_string(),
        };
        div()
            .flex()
            .flex_col()
            .gap(BASE_02)
            .children(
                headline
                    .label
                    .clone()
                    .map(|label| small(label, theme.text_muted)),
            )
            .child(
                value
                    .id("headline")
                    .role(Role::Status)
                    .aria_label(SharedString::from(name))
                    .font_features(tabular_figures())
                    // Sets the hero apart from the lines that explain it.
                    .when(headline.emphasized, |value| value.mb(BASE_02))
                    .child(headline.value.clone()),
            )
            .children(
                [
                    ("hint", &headline.hint),
                    ("lunch", &headline.lunch),
                    ("qualifier", &headline.qualifier),
                ]
                .into_iter()
                .filter_map(|(id, text)| {
                    let text = text.clone()?;
                    Some(small(readable(id, text), theme.text_muted))
                }),
            )
    }

    fn worked(&self, worked: &Worked, theme: &Theme) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(BASE_06)
            .child(
                div()
                    .id("worked")
                    .role(Role::Group)
                    .aria_label(SharedString::from(match &worked.target {
                        Some(target) => self.language.format(
                            Key::FlyoutWorkedAccessibleFormat,
                            &[&worked.label, &worked.worked, &target.duration],
                        ),
                        None => self.language.format(
                            Key::WorkdayPrimaryValueFormat,
                            &[&worked.label, &worked.worked],
                        ),
                    }))
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(BASE_08)
                    // Puts the label on the baseline of the larger worked time.
                    .child(
                        div()
                            .pb(BASE_02)
                            .child(small(worked.label.clone(), theme.text_muted)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .font_features(tabular_figures())
                            .child(
                                div()
                                    .type_style(text::WT_PRIMARY)
                                    .child(worked.worked.clone()),
                            )
                            .children(worked.target.as_ref().map(|target| {
                                div()
                                    .text_color(theme.text_muted)
                                    .child(format!("\u{a0}/\u{a0}{}", target.duration))
                            })),
                    ),
            )
            .children(worked.target.as_ref().map(|target| {
                div()
                    .id("progress")
                    .role(Role::ProgressIndicator)
                    .aria_label(self.text(Key::WorkdayProgressAutomationPropertiesName))
                    .aria_numeric_value(f64::from(target.progress * 100.))
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(theme.element_active)
                    .overflow_hidden()
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(target.progress))
                            .rounded(px(2.))
                            .bg(theme.accent),
                    )
            }))
            .children(self.day.credit.as_ref().map(|credit| {
                div()
                    .id("credit")
                    .role(Role::Group)
                    .aria_label(SharedString::from(self.language.format(
                        Key::WorkdayPrimaryValueFormat,
                        &[&credit.label, &credit.value],
                    )))
                    .flex()
                    .justify_between()
                    .gap(BASE_08)
                    .child(small(credit.label.clone(), theme.text_muted))
                    .child(
                        div()
                            .font_features(tabular_figures())
                            .child(small(credit.value.clone(), theme.text_muted)),
                    )
            }))
    }

    /// What scrolls between the header and the actions: the notices, the day and the callouts,
    /// down to the update. After Tab, it scrolls the part that took the focus into view, or
    /// the control that took it in a part taller than the body.
    pub(super) fn body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        theme: &Theme,
    ) -> Vec<AnyElement> {
        let mut parts = self.notices(window, cx, theme);
        parts.extend(
            self.source_callout(theme)
                .map(|callout| (Part::SourceCallout, callout)),
        );
        parts.extend(
            self.appointment_callout(theme)
                .map(|callout| (Part::AppointmentCallout, callout)),
        );
        parts.extend(
            self.counted_callout(theme)
                .map(|callout| (Part::CountedCallout, callout)),
        );
        parts.extend(
            self.trust_callout(theme)
                .map(|callout| (Part::TrustCallout, callout)),
        );
        parts.push((Part::Headline, self.headline(theme).into_any_element()));
        parts.extend(
            self.day
                .worked
                .as_ref()
                .map(|worked| (Part::Worked, self.worked(worked, theme).into_any_element())),
        );
        parts.extend(self.announcements(theme));
        parts.extend(
            self.update_callout(theme, cx)
                .map(|callout| (Part::UpdateCallout, callout)),
        );
        let (shown, parts): (Vec<Part>, Vec<AnyElement>) = parts.into_iter().unzip();
        while self.body_parts.len() < parts.len() {
            self.body_parts.push(cx.focus_handle());
        }
        self.refocus_after_closing(shown, window, cx);
        if std::mem::take(&mut self.reveal_focus)
            && let Some(focused) = self
                .body_parts
                .iter()
                .position(|part| part.contains_focused(window, cx))
            && let Some(part) = self.body_scroll.bounds_for_item(focused)
        {
            let control = self.focused_control(window, cx);
            let mut offset = self.body_scroll.offset();
            offset.y = revealing_offset(self.body_scroll.bounds(), offset.y, part, control);
            self.body_scroll.set_offset(offset);
        }
        self.body_controls.borrow_mut().clear();
        parts
            .into_iter()
            .zip(&self.body_parts)
            .map(|(part, handle)| {
                div()
                    .track_focus(handle)
                    // Clicked, the part would take the focus: it has no node for a screen reader.
                    .on_any_mouse_down(|_, window, _| window.prevent_default())
                    .child(part)
                    .into_any_element()
            })
            .collect()
    }

    /// After a callout closed with the focus in it, the next part takes the focus: its first
    /// control, or the first after it. The next part is the first still shown of those that
    /// followed the callout, whatever notice came or went with the day. The tab order is the one of the frame drawn without the
    /// callout, so the focus moves on the next frame. A callout the day still shows once it came
    /// back keeps the focus where it is.
    fn refocus_after_closing(
        &mut self,
        shown: Vec<Part>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shown_parts = shown;
        let Some(closing) = self.closing.take_if(|closing| closing.answered) else {
            return;
        };
        let Some((closed, after)) = closing
            .parts
            .get(closing.part..)
            .and_then(<[Part]>::split_first)
        else {
            return;
        };
        if self.shown_parts.contains(closed) {
            return;
        }
        let next = after
            .iter()
            .find_map(|part| self.shown_parts.iter().position(|shown| shown == part))
            .unwrap_or(self.shown_parts.len().saturating_sub(1));
        let part = self.body_parts[next].clone();
        let flyout = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            window.focus(&part, cx);
            window.focus_next(cx);
            flyout
                .update(cx, |flyout, cx| {
                    flyout.reveal_focus = true;
                    cx.notify();
                })
                .ok();
        });
    }

    /// Where the control of the body that holds the focus was last laid out, unscrolled.
    pub(super) fn focused_control(&self, window: &Window, cx: &App) -> Option<Bounds<Pixels>> {
        self.body_controls
            .borrow()
            .iter()
            .find(|(control, _)| control.contains_focused(window, cx))
            .map(|&(_, bounds)| bounds)
    }

    /// `element`, a control of the body that Tab can bring into view: see [`revealing_offset`].
    /// `id` keeps the control's focus from one frame to the next, so no two controls the body
    /// shows at once share one.
    pub(super) fn reveal_on_tab(&self, id: &'static str, element: impl IntoElement) -> AnyElement {
        BodyControl {
            id,
            laid_out: self.body_controls.clone(),
            body: self.body_scroll.clone(),
            element: element.into_any_element(),
        }
        .into_any_element()
    }
}

/// The offset that scrolls the body laid out at `body`, now scrolled by `offset`, the least that
/// shows `part` with [`BODY_PADDING`] on both sides. When the body cannot hold the part, it shows
/// the part's focused `control` instead; from its top when the body cannot hold that either. The
/// part and the control are where they were laid out, unscrolled.
fn revealing_offset(
    body: Bounds<Pixels>,
    offset: Pixels,
    part: Bounds<Pixels>,
    control: Option<Bounds<Pixels>>,
) -> Pixels {
    let shown = match control {
        Some(control) if part.size.height + BODY_PADDING * 2. > body.size.height => control,
        _ => part,
    };
    let (top, bottom) = (shown.top() - BODY_PADDING, shown.bottom() + BODY_PADDING);
    if top + offset < body.top() || bottom - top > body.size.height {
        body.top() - top
    } else if bottom + offset > body.bottom() {
        body.bottom() - bottom
    } else {
        offset
    }
}

/// Each control of the body with its focus and where it was laid out, unscrolled: unlike where it
/// was painted, that stays true when the body scrolls before the next render reads it.
pub(super) type LaidOutControls = Rc<RefCell<Vec<(FocusHandle, Bounds<Pixels>)>>>;

/// A control of the body, which adds where it was laid out to `laid_out`.
#[derive(IntoElement)]
struct BodyControl {
    id: &'static str,
    laid_out: LaidOutControls,
    body: ScrollHandle,
    element: AnyElement,
}

impl RenderOnce for BodyControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            laid_out,
            body,
            element,
        } = self;
        // GPUI keeps the focus handle it makes for a control's `tab_index` to itself, so this one
        // tracks the focus around it.
        let focus = window
            .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        div()
            .flex()
            .track_focus(&focus)
            .on_children_prepainted(move |bounds, _, _| {
                if let Some(&control) = bounds.first() {
                    let unscrolled = control - gpui::point(px(0.), body.offset().y);
                    laid_out.borrow_mut().push((focus.clone(), unscrolled));
                }
            })
            .child(element)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What runs from `top` to `bottom`, unscrolled, in a body as wide as these tests need.
    fn span(top: f32, bottom: f32) -> Bounds<Pixels> {
        Bounds::new(
            gpui::point(px(0.), px(top)),
            gpui::size(px(300.), px(bottom - top)),
        )
    }

    /// A body that shows from 100 to 300: a part up to 168 tall fits with the 16 of
    /// [`BODY_PADDING`] on both sides.
    fn body() -> Bounds<Pixels> {
        span(100., 300.)
    }

    #[test]
    fn a_part_the_body_already_shows_stays_where_it_is() {
        let part = span(150., 200.);
        assert_eq!(revealing_offset(body(), px(-20.), part, None), px(-20.));
        let control = Some(span(160., 190.));
        assert_eq!(revealing_offset(body(), px(-20.), part, control), px(-20.));
    }

    #[test]
    fn a_part_above_the_body_scrolls_down_to_its_padding_under_the_top() {
        // Shown from 20, revealed from 116.
        let part = span(120., 170.);
        assert_eq!(revealing_offset(body(), px(-100.), part, None), px(-4.));
    }

    #[test]
    fn a_part_below_the_body_scrolls_up_to_its_padding_over_the_bottom() {
        // Revealed down to 284.
        let part = span(400., 450.);
        assert_eq!(revealing_offset(body(), px(0.), part, None), px(-166.));
    }

    #[test]
    fn a_part_taller_than_the_body_shows_its_focused_control_below() {
        let part = span(116., 616.);
        let control = Some(span(560., 592.));
        assert_eq!(revealing_offset(body(), px(0.), part, control), px(-308.));
    }

    #[test]
    fn a_part_taller_than_the_body_shows_its_focused_control_above() {
        let part = span(116., 616.);
        let control = Some(span(130., 162.));
        assert_eq!(revealing_offset(body(), px(-400.), part, control), px(-14.));
    }

    #[test]
    fn a_part_taller_than_the_body_without_a_focused_control_shows_from_its_top() {
        let part = span(116., 616.);
        assert_eq!(revealing_offset(body(), px(-250.), part, None), px(0.));
    }

    #[test]
    fn a_control_taller_than_the_body_shows_from_its_top() {
        let part = span(116., 716.);
        let control = Some(span(300., 520.));
        assert_eq!(revealing_offset(body(), px(-50.), part, control), px(-184.));
    }

    #[test]
    fn an_arriving_notice_fades_in_over_its_entrance_while_it_rises_into_place() {
        let start = notice_pose(0.);
        assert_eq!((start.opacity, start.offset), (0., px(4.)));
        let early = notice_pose(0.15);
        assert!(
            (early.opacity - 0.556781).abs() < 1e-4,
            "{} is not 0.556781",
            early.opacity
        );
        let end = notice_pose(1.);
        assert_eq!((end.opacity, end.offset), (1., px(0.)));
    }

    #[test]
    fn a_failed_refresh_shows_under_any_answer_but_one_that_asks_for_a_refresh() {
        let failure = Notice::refresh_pending("WorkTime n’a pas pu actualiser les données.");
        let pending = Notice::refresh_pending("Réessayez l’actualisation.");
        let success = Notice::success("09:00–10:00 compté comme travail (1 h).");
        assert_eq!(shown_refresh_failure(None, Some(&failure)), Some(&failure));
        assert_eq!(
            shown_refresh_failure(Some(&success), Some(&failure)),
            Some(&failure)
        );
        assert_eq!(shown_refresh_failure(Some(&pending), Some(&failure)), None);
        assert_eq!(shown_refresh_failure(Some(&success), None), None);
    }
}
