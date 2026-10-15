//! Main view of one pull request (a click in the PRs tab): header, CI /
//! review / conversation cards, the description, its commits and its diff
//! against the base branch, like a small window onto the PR page.

use std::collections::HashSet;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::DiffFile as UiDiffFile;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use mugetsu_forge::{CheckState, PrDetails, PrState, PullRequest, ReviewDecision};
use std::time::Duration;

use super::RepoView;
use super::toolbar::pr_color;
use crate::i18n::tr;
use crate::{fmt, theme};

/// What the PR view loads in the background.
#[derive(Default)]
pub struct PrPage {
    pub number: u64,
    pub details: Option<Result<PrDetails, String>>,
    /// Files of `base...head`; `None` while loading.
    pub files: Option<Result<usize, String>>,
    /// Animation key of the last open.
    pub seq: usize,
}

fn ink() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.93)
}
fn soft() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.5)
}

impl RepoView {
    /// Local ref for a branch of a PR: the local branch, else the remote one.
    fn pr_ref(&self, branch: &str) -> Option<String> {
        let refs = &self.data.refs;
        if refs.branches.iter().any(|b| b.name == branch) {
            return Some(branch.to_string());
        }
        let remote = super::sidebar::default_remote(refs)?;
        let name = format!("{remote}/{branch}");
        refs.remote_branches
            .iter()
            .any(|b| b.name == name)
            .then_some(name)
    }

    fn focused_pr(&self) -> Option<PullRequest> {
        let n = self.pr_page.as_ref()?.number;
        self.pr_list.iter().find(|p| p.number == n).cloned()
    }

    /// Shows PR `number` in the main view and loads its description and diff.
    pub fn open_pr(&mut self, number: u64, cx: &mut Context<Self>) {
        self.menu_open = false;
        self.celebrate_seq += 1;
        self.pr_page = Some(PrPage {
            number,
            seq: self.celebrate_seq,
            ..Default::default()
        });
        self.set_mode(super::Mode::Pr, cx);
        let Some(pr) = self.focused_pr() else {
            return;
        };
        let forge = self.data.forge.clone();
        let range = match (self.pr_ref(&pr.base), self.pr_ref(&pr.head)) {
            (Some(base), Some(head)) => Ok(format!("{base}...{head}")),
            _ => Err(tr(
                "Branche absente en local : faites un fetch.",
                "Branch not available locally: fetch first.",
            )
            .to_string()),
        };
        let repo = self.repo.clone();
        self.pr_diff.update(cx, |d, cx| d.set_files(Vec::new(), cx));
        self._pr_task = cx.spawn(async move |this, cx| {
            let (details, files) = cx
                .background_spawn(async move {
                    let details = forge.map(|f| crate::pr::details(&f, number));
                    let files = range.and_then(|range| {
                        repo.git()
                            .run(["diff", "--no-ext-diff", "--patch", "--find-renames", &range])
                            .map_err(|e| e.to_string())
                            .map(|patch| UiDiffFile::parse(&patch).unwrap_or_default())
                    });
                    (details, files)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this.pr_page.as_mut().filter(|p| p.number == number) else {
                    return;
                };
                page.details = details;
                page.files = Some(files.as_ref().map(|f| f.len()).map_err(Clone::clone));
                if let Ok(files) = files {
                    this.pr_diff.update(cx, |d, cx| d.set_files(files, cx));
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Commits of the PR branch not on its base, newest first.
    fn pr_commits(&self, pr: &PullRequest) -> Vec<usize> {
        let history = &self.data.history;
        let row = |r: Option<String>| {
            let r = r?;
            let refs = &self.data.refs;
            refs.branches
                .iter()
                .chain(&refs.remote_branches)
                .find(|b| b.name == r)
                .and_then(|b| history.index.get(&b.target).copied())
        };
        let Some(head) = row(self.pr_ref(&pr.head)) else {
            return Vec::new();
        };
        let base: HashSet<usize> = row(self.pr_ref(&pr.base))
            .map(|b| super::compact::walk_rows(history, b, &HashSet::new(), usize::MAX))
            .unwrap_or_default()
            .into_iter()
            .collect();
        super::compact::walk_rows(history, head, &base, 200)
    }

    pub(super) fn render_pr_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let (Some(page), Some(pr)) = (self.pr_page.as_ref(), self.focused_pr()) else {
            return v_flex()
                .id("pr-view")
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_sm()
                .text_color(soft())
                .child(Icon::new(Lucide::GitPullRequest).large().text_color(soft()))
                .child(tr(
                    "Cette PR n'est plus listée.",
                    "This PR is no longer listed.",
                ))
                .into_any_element();
        };
        let seq = page.seq;
        let commits = self.pr_commits(&pr);
        let header = self.pr_header(&pr, page, cx);
        let forge = self
            .data
            .forge
            .as_ref()
            .map_or("GitHub", |f| f.kind.label());
        let cards = status_cards(&pr, page.details.as_ref(), forge);
        let body = description(page.details.as_ref());
        let now = fmt::now();
        let commit_rows = commits.iter().enumerate().map(|(i, &r)| {
            let c = &self.data.history.commits[r];
            let id = c.id;
            h_flex()
                .id(("pr-commit", i))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .hover(|s| s.bg(theme::raised()))
                .child(crate::avatars::avatar(&c.email, &c.author, 16., cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_color(ink()).child(c.summary.clone()))
                        .child(div().text_color(soft()).child(format!(
                            "{} · {}",
                            c.short_id(),
                            fmt::relative_time(c.time, now)
                        ))),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_mode(super::Mode::History, cx);
                    this.select_commit(id, true, cx);
                }))
        });
        let files_note: Option<String> = match &page.files {
            None => Some(tr("Calcul du diff…", "Computing the diff…").into()),
            Some(Err(e)) => Some(e.clone()),
            Some(Ok(0)) => Some(tr("Aucun changement.", "No changes.").into()),
            Some(Ok(_)) => None,
        };
        v_flex()
            .id("pr-view")
            .test_support()
            .size_full()
            .child(
                v_flex()
                    .flex_none()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .gap_3()
                    .child(header)
                    .child(cards)
                    .child(body)
                    .with_animation(
                        ("pr-view-in", seq),
                        Animation::new(Duration::from_millis(360)),
                        |el, t| {
                            let k = 1. - (1. - t).powi(3);
                            el.opacity(k).mt(px(10. * (1. - k)))
                        },
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .child(
                        v_flex()
                            .id("pr-commits")
                            .w(px(280.))
                            .flex_none()
                            .h_full()
                            .overflow_y_scroll()
                            .p_1()
                            .border_r_1()
                            .border_color(theme::hairline())
                            .child(div().px_2().py_1().text_xs().text_color(soft()).child(trf!(
                                "{} commit(s)",
                                "{} commit(s)",
                                commits.len()
                            )))
                            .children(commit_rows),
                    )
                    .child(div().flex_1().min_w_0().h_full().p_1().map(|s| {
                        match files_note {
                            Some(note) => s.child(
                                div()
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_sm()
                                    .text_color(soft())
                                    .child(note),
                            ),
                            None => s.child(
                                gpui_kit::component::diff::Diff::new(&self.pr_diff)
                                    .line_number(true),
                            ),
                        }
                    })),
            )
            .into_any_element()
    }

    fn pr_header(&self, pr: &PullRequest, page: &PrPage, cx: &mut Context<Self>) -> AnyElement {
        let color = pr_color(pr.state);
        let (state, icon) = match pr.state {
            PrState::Open => (tr("Ouverte", "Open"), Lucide::GitPullRequest),
            PrState::Draft => (tr("Brouillon", "Draft"), Lucide::GitPullRequestDraft),
            PrState::Merged => (tr("Mergée", "Merged"), Lucide::GitMerge),
            PrState::Closed => (tr("Fermée", "Closed"), Lucide::GitPullRequestClosed),
        };
        let size = page
            .details
            .as_ref()
            .and_then(|d| d.as_ref().ok())
            .map(|d| (d.additions, d.deletions, d.changed_files));
        let url = pr.url.clone();
        let head = pr.head.clone();
        let review_pr = pr.clone();
        let root = self.repo.root().to_path_buf();
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_1()
                            .px_2()
                            .h(px(22.))
                            .rounded_full()
                            .bg(theme::alpha(color, 0.18))
                            .text_color(color)
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(Icon::new(icon).xsmall())
                            .child(state),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_lg()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(ink())
                            .child(pr.title.clone()),
                    )
                    .child(div().text_color(soft()).child(format!("#{}", pr.number))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(soft())
                    .child(crate::avatars::pr_author(pr, 18., cx))
                    .child(div().text_color(ink()).child(pr.author.clone()))
                    .child(tr("veut merger", "wants to merge"))
                    .child(branch_pill(&pr.head))
                    .child(Icon::new(Lucide::ArrowRight).xsmall())
                    .child(branch_pill(&pr.base))
                    .when_some(size, |s, (add, del, files)| {
                        s.child(div().text_color(theme::lime()).child(format!("+{add}")))
                            .child(div().text_color(theme::red()).child(format!("−{del}")))
                            .child(trf!("{files} fichier(s)", "{files} file(s)"))
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("pr-show-branch")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Eye))
                            .label(tr("Montrer la branche", "Show branch"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.show_pr_branch(&head, cx)),
                            ),
                    )
                    .child(
                        Button::new("pr-ai-review")
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::Sparkles))
                            .label(tr("Review IA", "AI review"))
                            .on_click(move |_, window, cx| {
                                if let Err(e) = crate::ai_review::launch(&root, &review_pr) {
                                    gpui_kit::component::WindowExt::push_notification(
                                        window,
                                        gpui_kit::component::notification::Notification::error(
                                            format!("{e:#}"),
                                        ),
                                        cx,
                                    );
                                }
                            }),
                    )
                    .child(
                        Button::new("pr-open-github")
                            .outline()
                            .xsmall()
                            .icon(Icon::new(Lucide::ExternalLink))
                            .label("GitHub")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    ),
            )
            .into_any_element()
    }
}

fn branch_pill(name: &str) -> AnyElement {
    div()
        .px_1p5()
        .rounded_md()
        .bg(theme::alpha(theme::accent(), 0.12))
        .text_color(theme::accent())
        .child(name.to_string())
        .into_any_element()
}

/// CI, review and conversations, side by side.
fn status_cards(
    pr: &PullRequest,
    details: Option<&Result<PrDetails, String>>,
    forge: &str,
) -> AnyElement {
    let card = |id: &'static str, icon: Lucide, color: Hsla, title: String, sub: String| {
        h_flex()
            .id(id)
            .test_support()
            .flex_1()
            .min_w_0()
            .p_2p5()
            .gap_2p5()
            .rounded_xl()
            .border_1()
            .border_color(theme::alpha(color, 0.3))
            .bg(theme::alpha(color, 0.07))
            .child(
                div()
                    .size(px(30.))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(color, 0.18))
                    .child(Icon::new(icon).small().text_color(color)),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(ink())
                            .truncate()
                            .child(title),
                    )
                    .child(div().text_xs().text_color(soft()).truncate().child(sub)),
            )
    };
    let s = pr.status.clone().unwrap_or_default();
    let ci = match s.checks {
        Some(CheckState::Passing) => card(
            "pr-ci",
            Lucide::CircleCheck,
            theme::lime(),
            tr("CI OK", "CI passing").into(),
            trf!("{} checks", "{} checks", s.passed),
        ),
        Some(CheckState::Failing) => card(
            "pr-ci",
            Lucide::CircleX,
            theme::red(),
            tr("CI en échec", "CI failing").into(),
            s.failing.join(", "),
        ),
        Some(CheckState::Pending) => card(
            "pr-ci",
            Lucide::LoaderCircle,
            theme::amber(),
            tr("CI en cours", "CI running").into(),
            trf!("{} en attente", "{} pending", s.pending),
        ),
        None => card(
            "pr-ci",
            Lucide::CircleDashed,
            theme::muted(),
            tr("Pas de CI", "No checks").into(),
            if pr.status.is_none() {
                trf!("Token {forge} requis", "{forge} token needed")
            } else {
                String::new()
            },
        ),
    };
    let review = match s.review {
        Some(ReviewDecision::Approved) => card(
            "pr-review",
            Lucide::UserCheck,
            theme::lime(),
            tr("Approuvée", "Approved").into(),
            String::new(),
        ),
        Some(ReviewDecision::ChangesRequested) => card(
            "pr-review",
            Lucide::CircleAlert,
            theme::red(),
            tr("Changements demandés", "Changes requested").into(),
            String::new(),
        ),
        _ => card(
            "pr-review",
            Lucide::Eye,
            theme::violet(),
            tr("Review en attente", "Awaiting review").into(),
            pr.requested_reviewers.join(", "),
        ),
    };
    let comments = details
        .and_then(|d| d.as_ref().ok())
        .map(|d| d.comments)
        .unwrap_or_default();
    let talk = card(
        "pr-talk",
        Lucide::MessagesSquare,
        if s.unresolved > 0 {
            theme::amber()
        } else {
            theme::muted()
        },
        if s.unresolved > 0 {
            trf!(
                "{} conversation(s) ouverte(s)",
                "{} open conversation(s)",
                s.unresolved
            )
        } else {
            tr("Conversations résolues", "Conversations resolved").into()
        },
        trf!("{comments} commentaire(s)", "{comments} comment(s)"),
    );
    h_flex()
        .w_full()
        .gap_2()
        .child(ci)
        .child(review)
        .child(talk)
        .into_any_element()
}

/// The PR description (Markdown), height-capped and scrollable.
fn description(details: Option<&Result<PrDetails, String>>) -> AnyElement {
    let inner: AnyElement = match details {
        None => div()
            .text_xs()
            .text_color(soft())
            .child(tr(
                "Chargement de la description…",
                "Loading the description…",
            ))
            .with_animation(
                "pr-desc-loading",
                Animation::new(Duration::from_millis(900))
                    .repeat()
                    .with_easing(gpui_kit::pulsating_between(0.35, 1.)),
                |el, t| el.opacity(t),
            )
            .into_any_element(),
        Some(Err(e)) => div()
            .text_xs()
            .text_color(soft())
            .child(trf!(
                "Description indisponible : {e}",
                "Description unavailable: {e}"
            ))
            .into_any_element(),
        Some(Ok(d)) if d.body.trim().is_empty() => div()
            .text_xs()
            .text_color(soft())
            .child(tr("Pas de description.", "No description."))
            .into_any_element(),
        Some(Ok(d)) => gpui_kit::component::text::TextView::markdown(
            SharedString::from("pr-body"),
            d.body.clone(),
        )
        .into_any_element(),
    };
    let labels = details
        .and_then(|d| d.as_ref().ok())
        .map(|d| d.labels.clone())
        .unwrap_or_default();
    v_flex()
        .id("pr-description")
        .w_full()
        .max_h(px(200.))
        .overflow_y_scroll()
        .p_3()
        .gap_2()
        .rounded_xl()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::alpha(gpui_kit::white(), 0.028))
        .text_sm()
        .when(!labels.is_empty(), |s| {
            s.child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .children(labels.into_iter().map(|l| {
                        div()
                            .px_1p5()
                            .rounded_full()
                            .text_xs()
                            .bg(theme::alpha(theme::violet(), 0.16))
                            .text_color(theme::violet())
                            .child(l)
                    })),
            )
        })
        .child(inner)
        .into_any_element()
}
