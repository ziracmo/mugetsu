//! A floating card showing a long, multi-step operation's progress (the
//! "Checkout here" of a worktree's branch): spinner, current step, a
//! segmented bar and the time spent.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};

use super::RepoView;
use crate::theme;

/// Shared between the UI and the background operation.
#[derive(Clone, Default)]
pub struct ProgressHandle {
    /// 1-based index of the running step, 0 before the first.
    step: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
}

impl ProgressHandle {
    /// Marks step `index` (0-based) as running.
    pub fn step(&self, index: usize) {
        self.step.store(index + 1, Ordering::Relaxed);
    }

    pub fn finish(&self) {
        self.done.store(true, Ordering::Relaxed);
    }
}

pub struct OpProgress {
    pub title: String,
    pub steps: Vec<&'static str>,
    pub handle: ProgressHandle,
    started: Instant,
    seq: usize,
}

impl RepoView {
    /// Shows the card for an operation with these `steps` until it calls
    /// [`ProgressHandle::finish`]; returns the handle to give it.
    pub fn start_op_progress(
        &mut self,
        title: String,
        steps: Vec<&'static str>,
        cx: &mut Context<Self>,
    ) -> ProgressHandle {
        let handle = ProgressHandle::default();
        self.progress_seq += 1;
        self.op_progress = Some(OpProgress {
            title,
            steps,
            handle: handle.clone(),
            started: Instant::now(),
            seq: self.progress_seq,
        });
        let (watched, seq) = (handle.clone(), self.progress_seq);
        cx.spawn(async move |this, cx| {
            let mut last = usize::MAX;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let step = watched.step.load(Ordering::Relaxed);
                let done = watched.done.load(Ordering::Relaxed);
                let alive = this
                    .update(cx, |this, cx| {
                        let ours = this.op_progress.as_ref().is_some_and(|p| p.seq == seq);
                        if !ours {
                            return false;
                        }
                        if done {
                            this.op_progress = None;
                        }
                        // Steps, and the elapsed time once a second.
                        if done || step != last || this.op_progress_tick() {
                            cx.notify();
                        }
                        !done
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
                last = step;
            }
        })
        .detach();
        cx.notify();
        handle
    }

    #[cfg(test)]
    pub fn op_progress_title(&self) -> Option<String> {
        self.op_progress.as_ref().map(|p| p.title.clone())
    }

    fn op_progress_tick(&mut self) -> bool {
        let Some(p) = &self.op_progress else {
            return false;
        };
        let secs = p.started.elapsed().as_secs();
        std::mem::replace(&mut self.progress_shown_secs, secs) != secs
    }

    pub(super) fn render_op_progress(&self) -> Option<AnyElement> {
        let p = self.op_progress.as_ref()?;
        let current = p.handle.step.load(Ordering::Relaxed);
        let label = current
            .checked_sub(1)
            .and_then(|i| p.steps.get(i))
            .copied()
            .unwrap_or("…");
        let secs = p.started.elapsed().as_secs();
        let segments = (0..p.steps.len()).map(|i| {
            let segment = div().h(px(4.)).flex_1().rounded_full();
            if i + 1 < current {
                segment.bg(theme::accent()).into_any_element()
            } else if i + 1 == current {
                segment
                    .bg(theme::accent())
                    .with_animation(
                        ("op-progress-step", i),
                        Animation::new(Duration::from_millis(900)).repeat(),
                        |el, t| {
                            let k = 0.35 + 0.65 * (0.5 - 0.5 * (t * std::f32::consts::TAU).cos());
                            el.opacity(k)
                        },
                    )
                    .into_any_element()
            } else {
                segment
                    .bg(theme::alpha(theme::muted(), 0.25))
                    .into_any_element()
            }
        });
        let card = v_flex()
            .id("op-progress")
            .test_support()
            .w(px(360.))
            .p_3()
            .gap_2()
            .rounded_xl()
            .border_1()
            .border_color(theme::glass_edge())
            .bg(theme::popover())
            .shadow_lg()
            .child(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().small().color(theme::accent()))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(p.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child(label.to_string()),
                            ),
                    )
                    .when(secs >= 2, |s| {
                        s.child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(format!("{secs} s")),
                        )
                    }),
            )
            .child(h_flex().gap_1().children(segments))
            .with_animation(
                ("op-progress-in", p.seq),
                Animation::new(Duration::from_millis(260)).with_easing(gpui_kit::ease_out_quint()),
                |el, t| el.opacity(t).mt(px(10. * (1. - t))),
            );
        Some(
            h_flex()
                .absolute()
                .bottom_4()
                .left_0()
                .right_0()
                .justify_center()
                .child(card)
                .into_any_element(),
        )
    }
}
