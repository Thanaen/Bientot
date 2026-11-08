//! The plugin card of the flyout, after the counted time (FR-040): a header the app draws, with
//! the plugin's name and a « … » menu, the plugin's lines and at most two buttons. The other
//! cards are under « Afficher plutôt » in that menu, after « Masquer pour aujourd’hui », « Gérer
//! les plugins » and « Désactiver ».
//!
//! A card that changes is not announced: no live region, only the user's navigation reads it
//! (FR-042).

use bientot_plugin_api::PluginId;
use gpui::{
    Anchor, AnyElement, App, ClickEvent, Context, FocusHandle, Role, SharedString, Window, actions,
    anchored, deferred, div, prelude::*, px,
};

use super::{Closing, Flyout, body::Part, control, control_icon};
use crate::plugins::{self, PluginCards, card};
use crate::settings::Page;
use crate::settings_view;
use crate::strings::Key;
use crate::theme::spacing::*;
use crate::theme::{Theme, TypeStyled as _, motion, radius, size, text};
use crate::widgets::{self, caps, icon};

/// The ids of the card's buttons, which [`Flyout::reveal_on_tab`] keeps.
const ACTION_IDS: [&str; 2] = ["plugin-action-0", "plugin-action-1"];
const MENU_ID: &str = "plugin-menu";
const MENU_WIDTH: gpui::Pixels = px(200.);
/// The design system's ContextMenu option.
const ITEM_HEIGHT: gpui::Pixels = px(26.);
/// The keys of the open menu.
const MENU_CONTEXT: &str = "PluginMenu";
/// « Masquer pour aujourd’hui », « Gérer les plugins » and « Désactiver », before the other
/// cards.
const OWN_ITEMS: usize = 3;

actions!(worktime, [MenuNext, MenuPrevious, MenuFirst, MenuLast]);

/// ↓ and ↑, Tab and Maj+Tab go round the menu's items, Début and Fin to the first and the last.
pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("down", MenuNext, Some(MENU_CONTEXT)),
        gpui::KeyBinding::new("tab", MenuNext, Some(MENU_CONTEXT)),
        gpui::KeyBinding::new("up", MenuPrevious, Some(MENU_CONTEXT)),
        gpui::KeyBinding::new("shift-tab", MenuPrevious, Some(MENU_CONTEXT)),
        gpui::KeyBinding::new("home", MenuFirst, Some(MENU_CONTEXT)),
        gpui::KeyBinding::new("end", MenuLast, Some(MENU_CONTEXT)),
    ]);
}

/// The « … » menu of the card.
pub(super) struct Menu {
    open: bool,
    /// Starts over each time the menu opens.
    entrance: motion::Entrance,
    /// The « … » button, which takes the focus back when the menu closes.
    button: FocusHandle,
    /// One for each item, « Masquer pour aujourd’hui » first, which takes the focus when the
    /// menu opens, then the other cards after the [`OWN_ITEMS`].
    items: Vec<FocusHandle>,
    /// Whether the menu was open when the mouse went down on « … »: that press already closed
    /// it, through the click outside the menu, so the click that follows must not open it again.
    pressed_open: bool,
}

impl Menu {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            open: false,
            entrance: motion::Entrance::default(),
            button: cx.focus_handle(),
            items: (0..OWN_ITEMS).map(|_| cx.focus_handle()).collect(),
            pressed_open: false,
        }
    }
}

/// What [`Flyout::plugin_menu_state`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginMenuState {
    pub open: bool,
    /// The focus is on « … ».
    pub button_focused: bool,
    /// The focus is on one of the menu's items.
    pub item_focused: bool,
}

/// Where the open menu stands at `time`, a share of its entrance: it rises 4 px from its
/// button, which it opens above.
fn menu_pose(time: f32) -> motion::Entering {
    motion::slide_over(time, px(4.))
}

impl Flyout {
    /// The card the flyout shows, if a plugin has one.
    pub(super) fn plugin_card(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        theme: &Theme,
    ) -> Option<(Part, AnyElement)> {
        let cards = cx.try_global::<PluginCards>()?;
        let Some((id, shown)) = cards.shown() else {
            self.plugin_menu.open = false;
            return None;
        };
        let (id, shown) = (id.clone(), shown.clone());
        let others: Vec<(PluginId, SharedString)> = cards
            .more()
            .map(|(id, card)| (id.clone(), card.name.clone()))
            .collect();
        let (now, zone) = ((cards.clock)(), cards.zone.clone());
        let language = self.language;

        let menu_label: SharedString = language
            .format(Key::PluginCardMenuFormat, &[&shown.name])
            .into();
        let open = self.plugin_menu.open;
        let button = widgets::expandable(
            control(MENU_ID, menu_label.clone(), theme),
            open,
            cx.listener(|flyout, _: &(), window, cx| flyout.toggle_plugin_menu(window, cx)),
        )
        .track_focus(&self.plugin_menu.button)
        .size(size::BUTTON_DEFAULT)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(open, |button| button.bg(theme.element_active))
        .child(control_icon(MENU_ID, "ellipsis", theme))
        .capture_any_mouse_down(cx.listener(|flyout, _, _, _| {
            flyout.plugin_menu.pressed_open = flyout.plugin_menu.open;
        }))
        .on_click(cx.listener(|flyout, click: &ClickEvent, window, cx| {
            if click.is_keyboard() || !std::mem::take(&mut flyout.plugin_menu.pressed_open) {
                flyout.toggle_plugin_menu(window, cx);
            }
        }));
        let time = if open {
            self.plugin_menu.entrance.time(motion::MENU, window, cx)
        } else {
            self.plugin_menu.entrance.reset();
            1.
        };
        while self.plugin_menu.items.len() < others.len() + OWN_ITEMS {
            self.plugin_menu.items.push(cx.focus_handle());
        }
        let menu =
            open.then(|| self.plugin_menu(&id, &shown.name, menu_label, &others, time, theme, cx));
        let header =
            div()
                .flex()
                .items_center()
                .gap(BASE_06)
                .child(icon("puzzle", theme.text_muted).size(size::ICON_SM))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .type_style(text::LABEL_SM)
                        .text_color(theme.text_muted)
                        .child(shown.name.clone()),
                )
                // No taller than the name: the button overflows the header.
                .child(div().h_0().flex().items_center().child(
                    self.reveal_on_tab(MENU_ID, div().relative().child(button).children(menu)),
                ));

        let mut drawing = card::Drawing::new(now, &zone, language, theme);
        let lines: Vec<AnyElement> = shown
            .card
            .lines
            .iter()
            .map(|line| drawing.line(line))
            .collect();
        let actions = shown
            .card
            .actions
            .iter()
            .zip(ACTION_IDS)
            .map(|(action, action_id)| {
                let (plugin, action) = (id.clone(), action.clone());
                let button = settings_view::outlined(
                    action_id,
                    action.label.clone(),
                    size::BUTTON_MEDIUM,
                    theme,
                )
                .on_click(move |_, _, cx| plugins::press(&plugin, &action, cx));
                self.reveal_on_tab(action_id, button)
            })
            .collect::<Vec<_>>();

        let element = div()
            .id("plugin-card")
            .role(Role::Group)
            .aria_label(shown.name.clone())
            .flex()
            .flex_col()
            .gap(BASE_08)
            .p(BASE_12)
            .rounded(radius::MD)
            .border_1()
            .border_color(theme.border_variant)
            .child(header)
            .child(div().flex().flex_col().gap(BASE_04).children(lines))
            .when(!actions.is_empty(), |card| {
                card.child(
                    div()
                        .pt(BASE_04)
                        .flex()
                        .flex_wrap()
                        .gap(BASE_08)
                        .children(actions),
                )
            })
            .into_any_element();
        Some((Part::PluginCard, element))
    }

    #[allow(clippy::too_many_arguments)]
    fn plugin_menu(
        &self,
        plugin: &PluginId,
        name: &SharedString,
        label: SharedString,
        others: &[(PluginId, SharedString)],
        time: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hover = theme.element_hover;
        let focused = theme.border_focused;
        let item = |id: gpui::ElementId, label: SharedString, name: SharedString| {
            div()
                .id(id)
                .role(Role::MenuItem)
                .aria_label(name)
                .h(ITEM_HEIGHT)
                .px(BASE_08)
                .flex()
                .items_center()
                .rounded(radius::SM)
                .border_1()
                .border_color(gpui::transparent_black())
                .tab_index(0)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .focus_visible(move |style| style.border_color(focused))
                .child(div().truncate().child(label))
        };
        let hidden = plugin.clone();
        let hide_label: SharedString = self.text(Key::PluginCardHideToday).into();
        let hide = item("plugin-hide".into(), hide_label.clone(), hide_label)
            .track_focus(&self.plugin_menu.items[0])
            .on_click(cx.listener(move |flyout, _, window, cx| {
                flyout.take_plugin_card(&hidden, false, window, cx)
            }));
        let manage_label: SharedString = self.text(Key::PluginCardManage).into();
        let manage = item("plugin-manage".into(), manage_label.clone(), manage_label)
            .track_focus(&self.plugin_menu.items[1])
            .on_click(cx.listener(|flyout, _, _, cx| {
                flyout.plugin_menu.open = false;
                settings_view::open_on(Page::Plugins, cx);
                cx.notify();
            }));
        let turned_off = plugin.clone();
        let turn_off_label: SharedString = self
            .language
            .format(Key::PluginCardTurnOffFormat, &[name])
            .into();
        let turn_off = item(
            "plugin-turn-off".into(),
            turn_off_label.clone(),
            turn_off_label,
        )
        .track_focus(&self.plugin_menu.items[2])
        .on_click(cx.listener(move |flyout, _, window, cx| {
            flyout.take_plugin_card(&turned_off, true, window, cx)
        }));
        let others = (!others.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(BASE_02)
                .mt(BASE_04)
                .pt(BASE_08)
                .border_t_1()
                .border_color(theme.border_variant)
                // Past the items' border, so that the caption starts with their labels.
                .child(
                    div()
                        .px(BASE_08 + px(1.))
                        .pb(BASE_02)
                        .child(caps(self.text(Key::PluginCardOthers), theme)),
                )
                .children(others.iter().enumerate().map(|(index, (other, name))| {
                    let other = other.clone();
                    let spoken = self
                        .language
                        .format(Key::PluginCardShowFormat, &[name])
                        .into();
                    item(("plugin-other", index).into(), name.clone(), spoken)
                        .track_focus(&self.plugin_menu.items[index + OWN_ITEMS])
                        .on_click(cx.listener(move |flyout, _, window, cx| {
                            flyout.show_plugin_card(&other, window, cx)
                        }))
                }))
        });
        // Above the card, on its right edge: the card sits low in the flyout, and the day above
        // it leaves the room that the actions below do not.
        let menu = div()
            .id("plugin-menu-items")
            .role(Role::Menu)
            .aria_label(label)
            .key_context(MENU_CONTEXT)
            .on_action(cx.listener(|flyout, _: &MenuNext, window, cx| {
                flyout.move_in_plugin_menu(Step::Next, window, cx)
            }))
            .on_action(cx.listener(|flyout, _: &MenuPrevious, window, cx| {
                flyout.move_in_plugin_menu(Step::Previous, window, cx)
            }))
            .on_action(cx.listener(|flyout, _: &MenuFirst, window, cx| {
                flyout.move_in_plugin_menu(Step::First, window, cx)
            }))
            .on_action(cx.listener(|flyout, _: &MenuLast, window, cx| {
                flyout.move_in_plugin_menu(Step::Last, window, cx)
            }))
            .occlude()
            .map(|menu| menu_pose(time).apply(menu))
            .w(MENU_WIDTH)
            .p(BASE_04)
            .flex()
            .flex_col()
            .gap(BASE_02)
            .rounded(radius::LG)
            .border_1()
            .border_color(theme.border)
            .bg(theme.elevated_surface)
            .shadow(theme.shadow_elevated())
            .type_style(text::LABEL_MD)
            .on_mouse_down_out(cx.listener(|flyout, _, window, cx| {
                // A press on something that takes the focus keeps it; otherwise the focus would
                // stay on an item no longer drawn.
                let menu = &mut flyout.plugin_menu;
                menu.open = false;
                if menu.items.iter().any(|item| item.is_focused(window)) {
                    window.focus(&menu.button, cx);
                }
                cx.notify();
            }))
            .child(hide)
            .child(manage)
            .child(turn_off)
            .children(others);
        // From the button, out to the card's top right corner: the card's border and padding
        // to the right, and above, the padding and the half of the button past the name's line.
        let to_corner = px(1.) + BASE_12;
        let above = to_corner + (size::BUTTON_DEFAULT - text::LABEL_SM.line_height) / 2.;
        div()
            .absolute()
            .bottom_full()
            .right(-to_corner)
            .mb(above + BASE_04)
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomRight)
                        .snap_to_window_with_margin(BASE_08)
                        .child(menu),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    /// ↓, ↑, Début or Fin in the open menu: the focus goes round its items.
    fn move_in_plugin_menu(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let shown = cx
            .try_global::<PluginCards>()
            .map_or(0, |cards| cards.more().count())
            + OWN_ITEMS;
        let items = &self.plugin_menu.items[..shown.min(self.plugin_menu.items.len())];
        let current = items.iter().position(|item| item.is_focused(window));
        let next = step.from(current, items.len());
        window.focus(&items[next], cx);
    }

    /// Opens the card's « … » menu, as a click on it: for the screenshots and the tests.
    pub fn open_plugin_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.plugin_menu.open {
            self.toggle_plugin_menu(window, cx);
        }
    }

    /// Whether the card's « … » menu is open, and where the focus is in it: for the tests.
    pub fn plugin_menu_state(&self, window: &Window) -> PluginMenuState {
        let menu = &self.plugin_menu;
        PluginMenuState {
            open: menu.open,
            button_focused: menu.button.is_focused(window),
            item_focused: menu.items.iter().any(|item| item.is_focused(window)),
        }
    }

    fn toggle_plugin_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let menu = &mut self.plugin_menu;
        menu.open = !menu.open;
        if menu.open {
            let first = menu.items[0].clone();
            window.on_next_frame(move |window, cx| window.focus(&first, cx));
        }
        cx.notify();
    }

    /// Closes the open menu and gives the focus back to its button. Returns whether it was
    /// open, so that Échap closes the menu before the flyout.
    pub(super) fn close_plugin_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let was_open = std::mem::take(&mut self.plugin_menu.open);
        if was_open {
            window.focus(&self.plugin_menu.button, cx);
            cx.notify();
        }
        was_open
    }

    /// « Masquer pour aujourd’hui », or « Désactiver » when `turn_off`: the next card takes
    /// the place, its « … » the focus; with none, the focus goes to the part after the card, as
    /// after a closed callout.
    fn take_plugin_card(
        &mut self,
        plugin: &PluginId,
        turn_off: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.plugin_menu.open = false;
        let part = self
            .shown_parts
            .iter()
            .position(|part| *part == Part::PluginCard);
        if turn_off {
            plugins::turn(plugin, false, cx);
        } else {
            plugins::hide_for_today(plugin, cx);
        }
        let replaced = cx
            .try_global::<PluginCards>()
            .is_some_and(|cards| cards.shown().is_some());
        if replaced {
            window.focus(&self.plugin_menu.button, cx);
        } else if let Some(part) = part {
            self.closing = Some(Closing {
                part,
                parts: self.shown_parts.clone(),
                answered: true,
            });
        }
        cx.notify();
    }

    /// A card picked under « Afficher plutôt » takes the place, the focus on its « … ».
    fn show_plugin_card(&mut self, plugin: &PluginId, window: &mut Window, cx: &mut Context<Self>) {
        self.plugin_menu.open = false;
        plugins::show(plugin, cx);
        window.focus(&self.plugin_menu.button, cx);
        cx.notify();
    }
}

/// A key that moves the focus in the open menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Next,
    Previous,
    First,
    Last,
}

impl Step {
    /// The item this step focuses, out of `count`, from the `current` one if an item has the
    /// focus: round from the last to the first and back.
    fn from(self, current: Option<usize>, count: usize) -> usize {
        let last = count.saturating_sub(1);
        match (self, current) {
            (Step::First, _) | (Step::Next, None) => 0,
            (Step::Last, _) | (Step::Previous, None) => last,
            (Step::Next, Some(current)) => (current + 1) % count.max(1),
            (Step::Previous, Some(0)) => last,
            (Step::Previous, Some(current)) => current - 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_go_round_the_menu() {
        assert_eq!(Step::Next.from(Some(0), 3), 1);
        assert_eq!(Step::Next.from(Some(2), 3), 0);
        assert_eq!(Step::Previous.from(Some(0), 3), 2);
        assert_eq!(Step::Previous.from(Some(2), 3), 1);
        assert_eq!(Step::First.from(Some(2), 3), 0);
        assert_eq!(Step::Last.from(None, 3), 2);
        assert_eq!(Step::Next.from(None, 1), 0);
        assert_eq!(Step::Next.from(Some(0), 1), 0);
    }

    #[test]
    fn the_menu_rises_from_its_button_and_hides_the_card_from_its_first_frames() {
        let start = menu_pose(0.);
        assert_eq!((start.opacity, start.offset), (0., px(4.)));
        assert_eq!(menu_pose(motion::OPAQUE_AT).opacity, 1.);
    }
}
