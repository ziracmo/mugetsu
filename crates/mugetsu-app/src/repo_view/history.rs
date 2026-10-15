use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::diff::Diff;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, ClipboardItem, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use mugetsu_git::RefLabel;

use super::{Mode, RepoView, SelectNewer, SelectOlder};
use crate::fmt;
use crate::graph_view::{self, NodeStyle};
use crate::theme;

pub const ROW_HEIGHT: f32 = 26.;

fn label_chip(label: &RefLabel) -> AnyElement {
    let (text, color, icon) = match label {
        RefLabel::Branch { name, head } => (name.clone(), if *head { theme::cyan() } else { theme::lime() }, Lucide::GitBranch),
        RefLabel::Remote(name) => (name.clone(), theme::violet(), Lucide::Cloud),
        RefLabel::Tag(name) => (name.clone(), theme::amber(), Lucide::Tag),
        RefLabel::DetachedHead => ("HEAD".into(), theme::magenta(), Lucide::CircleDot),
    };
    let head = matches!(label, RefLabel::Branch { head: true, .. } | RefLabel::DetachedHead);
    h_flex()
        .flex_none()
        .max_w(px(220.))
        .h(px(18.))
        .gap_1()
        .px_1p5()
        .rounded_sm()
        .border_1()
        .border_color(theme::alpha(color, if head { 0.9 } else { 0.45 }))
        .bg(theme::alpha(color, if head { 0.2 } else { 0.1 }))
        .text_color(color)
        .text_xs()
        .child(Icon::new(icon).xsmall())
        .child(div().truncate().child(text))
        .into_any_element()
}

impl RepoView {
    fn render_wip_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.data.uncommitted();
        (n > 0).then(|| {
            h_flex()
                .id("wip")
                .h(px(28.))
                .flex_none()
                .px_3()
                .gap_2()
                .cursor_pointer()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(theme::alpha(theme::amber(), 0.06))
                .hover(|s| s.bg(theme::alpha(theme::amber(), 0.12)))
                .child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(theme::amber())
                        .with_animation(
                            "wip-pulse",
                            Animation::new(Duration::from_millis(1400)).repeat().with_easing(gpui_kit::pulsating_between(0.35, 1.)),
                            |el, t| el.opacity(t),
                        ),
                )
                .child(div().text_sm().text_color(theme::amber()).child(format!("{n} fichier(s) modifié(s) non commité(s)")))
                .child(div().flex_1())
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Voir les changements →"))
                .on_click(cx.listener(|this, _, _, cx| this.set_mode(Mode::Changes, cx)))
                .into_any_element()
        })
    }

    pub(super) fn render_history(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let data = self.data.clone();
        let selected = self.selected;
        let head = data.refs.head;
        let graph_width = graph_view::column_width(data.graph.max_width);
        let entity = cx.entity().downgrade();
        let now = fmt::now();
        let focused = self.history_focus.is_focused(window);

        let list = if data.history.is_empty() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(cx.theme().muted_foreground)
                .when(self.loading, |s| s.child(Spinner::new().color(theme::cyan())).child("Lecture de l'historique…"))
                .when(!self.loading, |s| s.child("Aucun commit"))
                .into_any_element()
        } else {
            uniform_list("history", data.history.len(), move |range, _, cx| {
                range
                    .map(|ix| {
                        let commit = &data.history.commits[ix];
                        let row = data.graph.row(ix);
                        let is_selected = selected == Some(commit.id);
                        let style = NodeStyle { is_head: head == Some(commit.id), is_merge: commit.parents.len() > 1, selected: is_selected };
                        let labels = data.decorations.get(&commit.id).map(|l| l.iter().map(label_chip).collect::<Vec<_>>()).unwrap_or_default();
                        let id = commit.id;
                        let entity = entity.clone();
                        h_flex()
                            .id(SharedString::from(commit.id.to_string()))
                            .h(px(ROW_HEIGHT))
                            .w_full()
                            .pr_3()
                            .gap_2()
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::raised()))
                            .when(is_selected, |s| {
                                s.bg(theme::alpha(theme::cyan(), if focused { 0.16 } else { 0.1 }))
                            })
                            .child(div().flex_none().w(graph_width).h_full().child(graph_view::graph_cell(row, style)))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1p5()
                                    .children(labels)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .when(commit.parents.len() > 1, |s| s.text_color(cx.theme().muted_foreground))
                                            .child(commit.summary.clone()),
                                    ),
                            )
                            .child(div().flex_none().w(px(130.)).truncate().text_xs().text_color(cx.theme().muted_foreground).child(commit.author.clone()))
                            .child(div().flex_none().w(px(56.)).text_xs().text_right().text_color(cx.theme().muted_foreground).child(fmt::relative_time(commit.time, now)))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(64.))
                                    .text_xs()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_color(theme::alpha(theme::cyan(), 0.7))
                                    .child(commit.id.to_hex_with_len(7).to_string()),
                            )
                            .on_click(move |_, window, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.history_focus.focus(window, cx);
                                    this.select_commit(id, false, cx);
                                });
                            })
                            .into_any_element()
                    })
                    .collect()
            })
            .size_full()
            .track_scroll(&self.history_scroll)
            .into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .children(self.render_wip_banner(cx))
            .child(
                div()
                    .id("history-list")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.history_focus)
                    .key_context("History")
                    .on_action(cx.listener(|this, _: &SelectNewer, _, cx| this.move_selection(-1, cx)))
                    .on_action(cx.listener(|this, _: &SelectOlder, _, cx| this.move_selection(1, cx)))
                    .child(list)
                    .child(Scrollbar::vertical(&self.history_scroll)),
            )
    }

    pub(super) fn render_commit_details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(commit) = self.selected.and_then(|id| self.data.history.row_of(&id)).map(|r| self.data.history.commits[r].clone()) else {
            return v_flex().size_full().items_center().justify_center().text_color(cx.theme().muted_foreground).child("Sélectionne un commit").into_any_element();
        };
        let sha = commit.id.to_string();
        let message = if self.commit_message.is_empty() { SharedString::from(commit.summary.clone()) } else { self.commit_message.clone() };
        let (subject, body) = message.split_once('\n').map_or((message.to_string(), String::new()), |(s, b)| (s.to_string(), b.trim().to_string()));
        v_flex()
            .size_full()
            .min_h_0()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_none()
                    .px_4()
                    .py_2()
                    .gap_1()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_base().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(subject))
                    .when(!body.is_empty(), |s| s.child(div().text_sm().text_color(cx.theme().muted_foreground).child(body)))
                    .child(
                        h_flex()
                            .gap_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{} <{}>", commit.author, commit.email))
                            .child(fmt::absolute_time(commit.time))
                            .child(
                                h_flex()
                                    .id("copy-sha")
                                    .gap_1()
                                    .cursor_pointer()
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_color(theme::cyan())
                                    .hover(|s| s.text_color(theme::magenta()))
                                    .child(Icon::new(Lucide::Copy).xsmall())
                                    .child(commit.id.to_hex_with_len(10).to_string())
                                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()))),
                            )
                            .child(div().flex_1())
                            .child(format!("{} fichier(s)", self.commit_files)),
                    ),
            )
            .child(div().flex_1().min_h_0().p_2().child(Diff::new(&self.commit_diff).line_number(true)))
            .into_any_element()
    }
}
