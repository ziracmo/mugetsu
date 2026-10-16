//! Small modal dialogs shared by views: a one-field prompt and a confirmation.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::{
    AppContext as _, Context, Focusable as _, ParentElement as _, Styled as _, Window, div, px,
};

use crate::i18n::tr;

/// Opens a one-field dialog; `action` gets the trimmed, non-empty value.
pub fn prompt<T: 'static>(
    title: String,
    placeholder: &'static str,
    initial: String,
    ok: &'static str,
    action: impl Fn(&mut T, String, &mut Window, &mut Context<T>) + 'static,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .default_value(initial)
    });
    let view = cx.entity().downgrade();
    let action = Rc::new(action);
    let focus_once = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if focus_once.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let (input_ok, view, action) = (input.clone(), view.clone(), action.clone());
        dialog
            .bg(crate::theme::sheet())
            .title(title.clone())
            .w(px(460.))
            .child(Input::new(&input))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))))
                    .child(DialogAction::new().child(Button::new("prompt-ok").primary().label(ok))),
            )
            .on_ok(move |_, window, cx| {
                let value = input_ok.read(cx).value().trim().to_string();
                if value.is_empty() {
                    return false;
                }
                let _ = view.update(cx, |this, cx| action(this, value, window, cx));
                true
            })
    });
}

/// Asks before a destructive action.
pub fn confirm<T: 'static>(
    title: String,
    description: String,
    ok: &'static str,
    action: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let view = cx.entity().downgrade();
    let action = Rc::new(action);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, action) = (view.clone(), action.clone());
        alert
            .bg(crate::theme::sheet())
            .title(title.clone())
            .description(description.clone())
            .confirm()
            .ok_text(ok)
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(tr("Annuler", "Cancel"))
            .on_ok(move |_, window, cx| {
                let _ = view.update(cx, |this, cx| action(this, window, cx));
                true
            })
    });
}

/// Like [`confirm`], with a checkbox under the description; `action` gets
/// whether it is ticked.
#[allow(clippy::too_many_arguments)]
pub fn confirm_with_option<T: 'static>(
    title: String,
    description: String,
    option: String,
    checked: bool,
    ok: &'static str,
    action: impl Fn(&mut T, bool, &mut Window, &mut Context<T>) + 'static,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let view = cx.entity().downgrade();
    let action = Rc::new(action);
    let ticked = Rc::new(Cell::new(checked));
    window.open_dialog(cx, move |dialog, _, _| {
        let (view, action, toggle, read) =
            (view.clone(), action.clone(), ticked.clone(), ticked.clone());
        dialog
            .bg(crate::theme::sheet())
            .title(title.clone())
            .w(px(460.))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(crate::theme::muted())
                            .child(description.clone()),
                    )
                    .child(
                        Checkbox::new("confirm-option")
                            .label(option.clone())
                            .checked(ticked.get())
                            .on_click(move |on: &bool, window, _| {
                                toggle.set(*on);
                                window.refresh();
                            }),
                    ),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))))
                    .child(DialogAction::new().child(Button::new("confirm-ok").danger().label(ok))),
            )
            .on_ok(move |_, window, cx| {
                let on = read.get();
                let _ = view.update(cx, |this, cx| action(this, on, window, cx));
                true
            })
    });
}
