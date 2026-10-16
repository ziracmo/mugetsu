//! The recap's summary as an image (1200 × 630, rendered at 2×): drawn as
//! SVG, rasterised with resvg (already in the app through GPUI) and the
//! system fonts.

use resvg::{tiny_skia, usvg};

use super::{Computed, day_names, delta_pct, week_label};
use crate::i18n::tr;

const W: f32 = 1200.;
const H: f32 = 630.;
const SCALE: f32 = 2.;
const FONT: &str =
    "Inter, 'SF Pro Display', 'Helvetica Neue', 'Segoe UI', Helvetica, Arial, sans-serif";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn hex(rgb: u32) -> String {
    format!("#{rgb:06x}")
}

fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// "↑ 32 %" and its colour, for a stat tile.
fn trend(cur: u64, prev: u64) -> (String, &'static str) {
    match delta_pct(cur, prev) {
        None if cur == 0 => (String::new(), "#888"),
        None => (tr("nouveau", "new").into(), "#9ccc7a"),
        Some(0) => (tr("= pareil", "= same").into(), "#999"),
        Some(p) if p > 0 => (format!("↑ {p} %"), "#9ccc7a"),
        Some(p) => (format!("↓ {} %", -p), "#e07a86"),
    }
}

/// The summary card as SVG.
pub fn svg(data: &Computed, team: bool, workspace: &str) -> String {
    let n = data.numbers(team);
    let (w, p) = (&n.week, &n.prev);
    let accent = hex(crate::settings::get().accent);
    let title = if team {
        tr("La semaine de l'équipe", "The team's week")
    } else {
        tr("Ma semaine", "My week")
    };
    let mut out = String::new();
    out.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="{FONT}">
<defs>
  <linearGradient id="glow" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{accent}" stop-opacity="0.35"/>
    <stop offset="0.65" stop-color="{accent}" stop-opacity="0"/>
  </linearGradient>
  <linearGradient id="bar" x1="0" y1="1" x2="0" y2="0">
    <stop offset="0" stop-color="{accent}" stop-opacity="0.45"/>
    <stop offset="1" stop-color="{accent}"/>
  </linearGradient>
</defs>
<rect width="{W}" height="{H}" fill="#0b0b0e"/>
<rect width="{W}" height="{H}" fill="url(#glow)"/>
<text x="64" y="84" fill="{accent}" font-size="22" font-weight="600">{kicker}</text>
<text x="64" y="140" fill="#f2f2f2" font-size="52" font-weight="700">{title}</text>
"##,
        kicker = esc(&format!("{workspace} · {}", week_label(data.span))),
        title = esc(title),
    ));

    // Four tiles.
    let tiles: [(&str, u64, u64, &str); 4] = [
        (
            tr("Commits", "Commits"),
            w.commits.into(),
            p.commits.into(),
            "",
        ),
        (
            tr("Lignes ajoutées", "Lines added"),
            w.additions,
            p.additions,
            "+",
        ),
        (
            tr("Jours actifs", "Active days"),
            w.active_days().into(),
            p.active_days().into(),
            "",
        ),
        (
            tr("PR mergées", "PRs merged"),
            n.prs.into(),
            n.prs_prev.into(),
            "",
        ),
    ];
    let (tw, gap, ty) = (252., 24., 184.);
    for (i, (label, cur, prev, prefix)) in tiles.iter().enumerate() {
        let x = 64. + i as f32 * (tw + gap);
        let (t, color) = trend(*cur, *prev);
        out.push_str(&format!(
            r##"<rect x="{x}" y="{ty}" width="{tw}" height="150" rx="18" fill="#ffffff" fill-opacity="0.045" stroke="#ffffff" stroke-opacity="0.09"/>
<text x="{lx}" y="{l1}" fill="#9a9a9a" font-size="18">{label}</text>
<text x="{lx}" y="{l2}" fill="#f2f2f2" font-size="48" font-weight="700">{prefix}{value}</text>
<text x="{lx}" y="{l3}" fill="{color}" font-size="18" font-weight="600">{t}</text>
"##,
            lx = x + 24.,
            l1 = ty + 40.,
            l2 = ty + 100.,
            l3 = ty + 132.,
            label = esc(label),
            value = compact(*cur),
            t = esc(&t),
        ));
    }

    // Commits per day.
    let (bx, by, bw, bh) = (64., 400., 640., 150.);
    let max = w.days.iter().copied().max().unwrap_or(0).max(1) as f32;
    let col = bw / 7.;
    let letters: Vec<String> = day_names()
        .iter()
        .map(|d| d.chars().take(3).collect())
        .collect();
    out.push_str(&format!(
        r##"<text x="{bx}" y="{y}" fill="#9a9a9a" font-size="18">{t}</text>
"##,
        y = by - 12.,
        t = esc(tr("Commits par jour", "Commits per day")),
    ));
    for (i, &v) in w.days.iter().enumerate() {
        let h = (v as f32 / max * (bh - 30.)).max(if v > 0 { 4. } else { 2. });
        let x = bx + i as f32 * col + 8.;
        let future = i >= data.days;
        let (fill, opacity) = if future {
            ("#ffffff", 0.05)
        } else if v as f32 == max && v > 0 {
            ("url(#bar)", 1.)
        } else {
            ("#ffffff", 0.25)
        };
        out.push_str(&format!(
            r##"<rect x="{x}" y="{y}" width="{cw}" height="{h}" rx="5" fill="{fill}" fill-opacity="{opacity}"/>
<text x="{cx}" y="{ly}" fill="#8a8a8a" font-size="15" text-anchor="middle">{label}</text>
"##,
            y = by + bh - 30. - h,
            cw = col - 16.,
            cx = x + (col - 16.) / 2.,
            ly = by + bh - 6.,
            label = esc(&letters[i]),
        ));
        if !future && v > 0 {
            out.push_str(&format!(
                r##"<text x="{cx}" y="{vy}" fill="#cfcfcf" font-size="15" text-anchor="middle">{v}</text>
"##,
                cx = x + (col - 16.) / 2.,
                vy = by + bh - 36. - h,
            ));
        }
    }

    // Top repositories, or the podium in team mode.
    let (rx, ry) = (760., 400.);
    let (heading, rows): (&str, Vec<(String, u32)>) = if team && w.people.len() > 1 {
        (
            tr("Top contributeurs", "Top contributors"),
            w.people
                .iter()
                .take(4)
                .map(|p| (p.name.clone(), p.commits))
                .collect(),
        )
    } else {
        (
            tr("Dépôts", "Repositories"),
            w.repos.iter().take(4).cloned().collect(),
        )
    };
    out.push_str(&format!(
        r##"<text x="{rx}" y="{y}" fill="#9a9a9a" font-size="18">{t}</text>
"##,
        y = ry - 12.,
        t = esc(heading),
    ));
    let top = rows.first().map_or(1, |r| r.1).max(1) as f32;
    for (i, (label, v)) in rows.iter().enumerate() {
        let y = ry + 14. + i as f32 * 34.;
        let label: String = label.chars().take(28).collect();
        out.push_str(&format!(
            r##"<text x="{rx}" y="{ty}" fill="#d6d6d6" font-size="17">{label}</text>
<text x="1136" y="{ty}" fill="#f2f2f2" font-size="17" font-weight="600" text-anchor="end">{v}</text>
<rect x="{rx}" y="{by}" width="376" height="5" rx="2.5" fill="#ffffff" fill-opacity="0.07"/>
<rect x="{rx}" y="{by}" width="{bw}" height="5" rx="2.5" fill="{accent}"/>
"##,
            ty = y + 4.,
            by = y + 12.,
            bw = (376. * *v as f32 / top).max(4.),
            label = esc(&label),
        ));
    }
    out.push_str(&format!(
        r##"<text x="1136" y="604" fill="#6a6a6a" font-size="16" text-anchor="end">Mugetsu</text>
<text x="64" y="604" fill="#6a6a6a" font-size="16">{}</text>
</svg>"##,
        esc(tr(
            "Comparé au même moment de la semaine dernière",
            "Compared with the same point last week"
        ))
    ));
    out
}

/// PNG bytes of the summary card.
pub fn render(data: &Computed, team: bool, workspace: &str) -> anyhow::Result<Vec<u8>> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(&svg(data, team, workspace), &opt)?;
    let (w, h) = ((W * SCALE) as u32, (H * SCALE) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| anyhow::anyhow!("empty image"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap.encode_png()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_png_of_the_summary() {
        let mut data = Computed {
            days: 3,
            ..Computed::default()
        };
        data.mine.week.commits = 12;
        data.mine.week.days = [4, 6, 2, 0, 0, 0, 0];
        data.mine.week.repos = vec![("mugetsu & <co>".into(), 12)];
        data.mine.prev.commits = 8;
        let svg = svg(&data, false, "Perso");
        assert!(svg.contains("mugetsu &amp; &lt;co&gt;"));
        assert!(svg.contains("↑ 50 %"));
        let png = render(&data, false, "Perso").unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
