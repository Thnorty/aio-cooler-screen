//! Renders the screen designs: a transparent 480x480 PNG the cooler draws on top of the
//! background media. Shapes use tiny-skia; text uses Windows fonts via ab_glyph.

use std::f32::consts::PI;
use std::sync::OnceLock;

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont, VariableFont};
use chrono::{DateTime, Local};
use image::RgbaImage;
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, Paint, PathBuilder, Pixmap, Point, RadialGradient, Rect, SpreadMode,
    Stroke, Transform,
};

use crate::config::{
    BigStatOptions, ClockOptions, ClockStats, CustomDesign, Design, FontChoice, GaugesOptions, Look, Position,
    StatValues, StatsConfig, TilesOptions, Widget, WidgetKind,
};
use crate::media::SCREEN_SIZE;
use crate::sensors::Stat;

const SIZE: f32 = SCREEN_SIZE as f32;
const CENTER: f32 = SIZE / 2.0;

type Rgb = [u8; 3];

// ---------------------------------------------------------------------------------------
// Fonts and text

struct Fonts {
    bold: FontVec,
    regular: FontVec,
}

fn load_font(path: &str, weight: Option<f32>) -> Option<FontVec> {
    let mut font = FontVec::try_from_vec(std::fs::read(path).ok()?).ok()?;
    if let Some(w) = weight {
        font.set_variation(b"wght", w);
    }
    Some(font)
}

fn first_font(candidates: &[(String, Option<f32>)]) -> Option<FontVec> {
    candidates.iter().find_map(|(p, w)| load_font(p, *w))
}

// Bundled OFL fonts are installed alongside the executable (not into Windows Fonts).
fn bundled_font(name: &str) -> String {
    std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|d| d.join("fonts").join(name)))
        .unwrap_or_else(|| std::path::PathBuf::from("fonts").join(name))
        .to_string_lossy().into_owned()
}

fn fonts(choice: FontChoice) -> Option<&'static Fonts> {
    static FONTS: [OnceLock<Option<Fonts>>; 12] = [const { OnceLock::new() }; 12];
    let idx = FontChoice::ALL.iter().position(|c| *c == choice).unwrap_or(0);
    FONTS[idx]
        .get_or_init(|| {
            const F: &str = r"C:\Windows\Fonts\";
            let (bold, regular): (Vec<(String, Option<f32>)>, Vec<(String, Option<f32>)>) = match choice {
                FontChoice::SegoeUi => (vec![(format!("{F}segoeuib.ttf"), None)], vec![(format!("{F}seguisb.ttf"), None)]),
                FontChoice::Bahnschrift => {
                    (vec![(format!("{F}bahnschrift.ttf"), Some(700.0))], vec![(format!("{F}bahnschrift.ttf"), Some(400.0))])
                }
                FontChoice::Arial => (vec![(format!("{F}arialbd.ttf"), None)], vec![(format!("{F}arial.ttf"), None)]),
                FontChoice::Consolas => (vec![(format!("{F}consolab.ttf"), None)], vec![(format!("{F}consola.ttf"), None)]),
                FontChoice::GoogleSans => (vec![(bundled_font("GoogleSans-Bold.ttf"), Some(700.0))], vec![(bundled_font("GoogleSans-Regular.ttf"), Some(400.0))]),
                FontChoice::GoogleSansFlex => (vec![(bundled_font("GoogleSansFlex-Bold.ttf"), Some(700.0))], vec![(bundled_font("GoogleSansFlex-Regular.ttf"), Some(400.0))]),
                FontChoice::Inter => (vec![(bundled_font("Inter-Bold.ttf"), Some(700.0))], vec![(bundled_font("Inter-Regular.ttf"), Some(400.0))]),
                FontChoice::Rajdhani => (vec![(bundled_font("Rajdhani-Bold.ttf"), Some(700.0))], vec![(bundled_font("Rajdhani-Regular.ttf"), Some(400.0))]),
                FontChoice::Orbitron => (vec![(bundled_font("Orbitron-Bold.ttf"), Some(700.0))], vec![(bundled_font("Orbitron-Regular.ttf"), Some(400.0))]),
                FontChoice::JetBrainsMono => (vec![(bundled_font("JetBrainsMono-Bold.ttf"), Some(700.0))], vec![(bundled_font("JetBrainsMono-Regular.ttf"), Some(400.0))]),
                FontChoice::RobotoCondensed => (vec![(bundled_font("RobotoCondensed-Bold.ttf"), Some(700.0))], vec![(bundled_font("RobotoCondensed-Regular.ttf"), Some(400.0))]),
                FontChoice::Oswald => (vec![(bundled_font("Oswald-Bold.ttf"), Some(700.0))], vec![(bundled_font("Oswald-Regular.ttf"), Some(400.0))]),
            };
            // Fall back to Segoe UI / Arial when a font is missing.
            let fallback_bold = [(format!("{F}segoeuib.ttf"), None), (format!("{F}arialbd.ttf"), None)];
            let fallback_regular = [(format!("{F}seguisb.ttf"), None), (format!("{F}arial.ttf"), None)];
            let bold_list: Vec<_> = bold.iter().chain(fallback_bold.iter()).cloned().collect();
            let regular_list: Vec<_> = regular.iter().chain(fallback_regular.iter()).cloned().collect();
            Some(Fonts { bold: first_font(&bold_list)?, regular: first_font(&regular_list)? })
        })
        .as_ref()
}

fn text_width(font: &FontVec, size: f32, text: &str) -> f32 {
    let sf = font.as_scaled(PxScale::from(size));
    let mut width = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            width += sf.kern(p, id);
        }
        width += sf.h_advance(id);
        prev = Some(id);
    }
    width
}

/// Source-over blend of `color` with coverage `alpha` (0..1) onto a straight-alpha pixel.
fn blend(img: &mut RgbaImage, x: i32, y: i32, color: [u8; 4], alpha: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let sa = alpha.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let dst = img.get_pixel_mut(x as u32, y as u32);
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    for i in 0..3 {
        dst[i] = ((color[i] as f32 * sa + dst[i] as f32 * da * (1.0 - sa)) / out_a).round() as u8;
    }
    dst[3] = (out_a * 255.0).round() as u8;
}

fn draw_text(img: &mut RgbaImage, font: &FontVec, size: f32, x: f32, baseline: f32, text: &str, color: [u8; 4]) {
    let scale = PxScale::from(size);
    let sf = font.as_scaled(scale);
    let mut caret = x;
    let mut prev = None;
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            caret += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, color, cov));
        }
        caret += sf.h_advance(id);
        prev = Some(id);
    }
}

/// A deferred text draw; text goes on top after all shapes are rasterised.
struct Text {
    bold: bool,
    size: f32,
    cx: f32,
    baseline: f32,
    text: String,
    color: [u8; 4],
}

/// Collects text for one design, applying the look's size scale.
struct Texts<'a> {
    look: &'a Look,
    items: Vec<Text>,
}

impl Texts<'_> {
    fn add(&mut self, bold: bool, size: f32, cx: f32, baseline: f32, text: impl Into<String>, color: [u8; 4]) {
        let scale = self.look.text_scale.clamp(60, 140) as f32 / 100.0;
        self.items.push(Text { bold, size: size * scale, cx, baseline, text: text.into(), color });
    }
}

// ---------------------------------------------------------------------------------------
// Shapes

fn paint(rgb: Rgb, alpha: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(rgb[0], rgb[1], rgb[2], alpha);
    p.anti_alias = true;
    p
}

/// A circular arc (angles in degrees, 0 = 3 o'clock, clockwise) as cubic segments.
fn arc_path(cx: f32, cy: f32, r: f32, start_deg: f32, end_deg: f32) -> Option<tiny_skia::Path> {
    let (a0, a1) = (start_deg.to_radians(), end_deg.to_radians());
    let segments = ((a1 - a0).abs() / (PI / 2.0)).ceil().max(1.0) as usize;
    let step = (a1 - a0) / segments as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let mut pb = PathBuilder::new();
    pb.move_to(cx + r * a0.cos(), cy + r * a0.sin());
    for i in 0..segments {
        let s = a0 + step * i as f32;
        let e = s + step;
        let (p0, p1) = ((s.cos(), s.sin()), (e.cos(), e.sin()));
        pb.cubic_to(
            cx + r * (p0.0 - k * p0.1),
            cy + r * (p0.1 + k * p0.0),
            cx + r * (p1.0 + k * p1.1),
            cy + r * (p1.1 - k * p1.0),
            cx + r * p1.0,
            cy + r * p1.1,
        );
    }
    pb.finish()
}

fn stroke_arc(pm: &mut Pixmap, r: f32, start: f32, end: f32, width: f32, rgb: Rgb, alpha: u8) {
    stroke_arc_at(pm, CENTER, CENTER, r, start, end, width, rgb, alpha);
}

#[allow(clippy::too_many_arguments)]
fn stroke_arc_at(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, start: f32, end: f32, width: f32, rgb: Rgb, alpha: u8) {
    if (end - start).abs() < 0.5 || r <= 0.0 {
        return;
    }
    if let Some(path) = arc_path(cx, cy, r, start, end) {
        let stroke = Stroke { width, line_cap: LineCap::Round, ..Default::default() };
        pm.stroke_path(&path, &paint(rgb, alpha), &stroke, Transform::identity(), None);
    }
}

fn fill_rounded_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, rgb: Rgb, alpha: u8) {
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint(rgb, alpha), FillRule::Winding, Transform::identity(), None);
    }
}

/// Darken the round screen area, more towards the edge (a soft vignette).
fn scrim(pm: &mut Pixmap, dim: u8) {
    if dim == 0 {
        return;
    }
    let a = dim.min(90) as f32 / 100.0;
    let stops = vec![
        GradientStop::new(0.0, Color::from_rgba(0.0, 0.0, 0.0, a * 0.8).unwrap_or(Color::BLACK)),
        GradientStop::new(1.0, Color::from_rgba(0.0, 0.0, 0.0, (a * 1.2).min(1.0)).unwrap_or(Color::BLACK)),
    ];
    let mut p = Paint::default();
    p.anti_alias = true;
    if let Some(shader) = RadialGradient::new(
        Point::from_xy(CENTER, CENTER),
        Point::from_xy(CENTER, CENTER),
        CENTER,
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        p.shader = shader;
    }
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, SIZE, SIZE) {
        pm.fill_rect(rect, &p, Transform::identity(), None);
    }
}

// ---------------------------------------------------------------------------------------
// Colours and formatting

fn lerp(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// Blue when cool, green, amber, then red when hot.
fn temp_color(t: f64) -> Rgb {
    const STOPS: [(f64, Rgb); 4] = [(40.0, [56, 189, 248]), (60.0, [74, 222, 128]), (75.0, [251, 191, 36]), (90.0, [244, 63, 94])];
    if t <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for w in STOPS.windows(2) {
        if t <= w[1].0 {
            return lerp(w[0].1, w[1].1, ((t - w[0].0) / (w[1].0 - w[0].0)) as f32);
        }
    }
    STOPS[3].1
}

fn stat_color(look: &Look, stat: Stat, v: Option<f64>) -> Rgb {
    match v {
        Some(t) if stat.is_temp() && look.temp_colors => temp_color(t),
        _ => look.accent,
    }
}

fn fraction(v: Option<f64>) -> f32 {
    (v.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32
}

fn format_value(stat: Stat, v: Option<f64>) -> String {
    match v {
        Some(v) if stat.is_temp() => format!("{v:.0}°"),
        Some(v) => format!("{v:.0}%"),
        None => "--".into(),
    }
}

fn rgba(rgb: Rgb, a: u8) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], a]
}

fn tile_label(stat: Stat) -> &'static str {
    match stat {
        Stat::CpuTemp => "CPU TEMP",
        Stat::CpuLoad => "CPU LOAD",
        Stat::GpuTemp => "GPU TEMP",
        Stat::GpuLoad => "GPU LOAD",
    }
}

fn partner(stat: Stat) -> Stat {
    match stat {
        Stat::CpuTemp => Stat::CpuLoad,
        Stat::CpuLoad => Stat::CpuTemp,
        Stat::GpuTemp => Stat::GpuLoad,
        Stat::GpuLoad => Stat::GpuTemp,
    }
}

// ---------------------------------------------------------------------------------------
// Designs

fn gauges(pm: &mut Pixmap, o: &GaugesOptions, v: &StatValues, t: &mut Texts) {
    let look = &o.look;
    let track = [255, 255, 255];
    let width = o.thickness.clamp(8, 36) as f32;
    let outer = 232.0 - width / 2.0;
    // Left half: CPU, filling bottom-to-top. Right half: GPU, mirrored.
    let sides = [(Stat::CpuTemp, Stat::CpuLoad, 112.0_f32, 248.0_f32, CENTER - 92.0), (Stat::GpuTemp, Stat::GpuLoad, 68.0, -68.0, CENTER + 92.0)];
    for (temp, load, start, end, cx) in sides {
        let span = end - start;
        stroke_arc(pm, outer, start, end, width, track, 34);
        stroke_arc(pm, outer, start, start + span * fraction(v.get(temp)), width, stat_color(look, temp, v.get(temp)), 255);
        if o.load_rings {
            let r = outer - width / 2.0 - 14.0;
            stroke_arc(pm, r, start, end, 8.0, track, 28);
            stroke_arc(pm, r, start, start + span * fraction(v.get(load)), 8.0, look.accent, 230);
        }
        if o.labels {
            t.add(false, 30.0, cx, 186.0, temp.label(), rgba(look.text, 170));
        }
        t.add(true, 76.0, cx, 268.0, format_value(temp, v.get(temp)), rgba(look.text, 255));
        if o.load_rings || o.labels {
            t.add(false, 28.0, cx, 312.0, format!("Load {}", format_value(load, v.get(load))), rgba(look.text, 190));
        }
    }
    if let Some(rect) = Rect::from_xywh(CENTER - 1.0, 170.0, 2.0, 150.0) {
        pm.fill_rect(rect, &paint(track, 40), Transform::identity(), None);
    }
}

fn tiles(pm: &mut Pixmap, o: &TilesOptions, v: &StatValues, t: &mut Texts, fonts: &Fonts) {
    let look = &o.look;
    let scale = look.text_scale.clamp(60, 140) as f32 / 100.0;
    let (label_size, value_size) = (28.0 * scale, 84.0 * scale);
    const PAD: f32 = 24.0;
    const GAP: f32 = 14.0;
    let h = 132.0 * scale.max(0.85);
    if o.show.is_empty() {
        return;
    }
    let cells: Vec<(Stat, String)> = o.show.iter().map(|&s| (s, format_value(s, v.get(s)))).collect();
    let tile_w = cells
        .iter()
        .map(|(s, val)| text_width(&fonts.bold, value_size, val).max(text_width(&fonts.regular, label_size, tile_label(*s))) + 2.0 * PAD)
        .fold(150.0, f32::max);
    let per_row = o.columns.clamp(1, 2) as usize;
    let rows: Vec<&[(Stat, String)]> = cells.chunks(per_row).collect();
    let block_h = rows.len() as f32 * h + (rows.len() as f32 - 1.0) * GAP;
    let top = match o.position {
        Position::Top => 64.0,
        Position::Center => (SIZE - block_h) / 2.0,
        Position::Bottom => SIZE - 64.0 - block_h,
    };
    let panel_alpha = (o.panel_opacity.min(100) as f32 * 2.55) as u8;
    for (ri, row) in rows.iter().enumerate() {
        let row_w = row.len() as f32 * tile_w + (row.len() as f32 - 1.0) * GAP;
        let y = top + ri as f32 * (h + GAP);
        for (ci, (stat, val)) in row.iter().enumerate() {
            let x = (SIZE - row_w) / 2.0 + ci as f32 * (tile_w + GAP);
            if o.panel {
                fill_rounded_rect(pm, x, y, tile_w, h, 20.0, [10, 12, 16], panel_alpha);
            }
            fill_rounded_rect(pm, x + tile_w / 2.0 - 22.0, y + 10.0, 44.0, 5.0, 2.5, stat_color(look, *stat, v.get(*stat)), 255);
            let cx = x + tile_w / 2.0;
            // Sizes are already scaled; add them unscaled.
            t.items.push(Text { bold: false, size: label_size, cx, baseline: y + h * 0.35, text: tile_label(*stat).into(), color: rgba(look.text, 180) });
            t.items.push(Text { bold: true, size: value_size, cx, baseline: y + h * 0.89, text: val.clone(), color: rgba(look.text, 255) });
        }
    }
}

fn big_stat(pm: &mut Pixmap, o: &BigStatOptions, v: &StatValues, t: &mut Texts) {
    let look = &o.look;
    let stat = o.stat;
    let value = v.get(stat);
    let color = stat_color(look, stat, value);
    // 270° ring with the gap at the bottom, plus tick marks every 10%.
    let (start, sweep) = (135.0_f32, 270.0_f32);
    if o.ring {
        stroke_arc(pm, 206.0, start, start + sweep, 18.0, [255, 255, 255], 34);
        stroke_arc(pm, 206.0, start, start + sweep * fraction(value), 18.0, color, 255);
    }
    if o.ticks {
        for i in 0..=10 {
            let a = (start + sweep * i as f32 / 10.0).to_radians();
            let (r0, r1) = (178.0, if i % 5 == 0 { 166.0 } else { 172.0 });
            let mut pb = PathBuilder::new();
            pb.move_to(CENTER + r0 * a.cos(), CENTER + r0 * a.sin());
            pb.line_to(CENTER + r1 * a.cos(), CENTER + r1 * a.sin());
            if let Some(path) = pb.finish() {
                let stroke = Stroke { width: 3.0, line_cap: LineCap::Round, ..Default::default() };
                pm.stroke_path(&path, &paint([255, 255, 255], 90), &stroke, Transform::identity(), None);
            }
        }
    }
    t.add(true, 150.0, CENTER, 290.0, format_value(stat, value), rgba(look.text, 255));
    t.add(false, 32.0, CENTER, 345.0, tile_label(stat), rgba(color, 255));
    if o.secondary {
        let other = partner(stat);
        t.add(false, 26.0, CENTER, 400.0, format!("{} {}", tile_label(other), format_value(other, v.get(other))), rgba(look.text, 170));
    }
}

/// The time as "14:09", or "2:09" plus "PM" in 12-hour mode.
fn time_parts(now: DateTime<Local>, h24: bool) -> (String, Option<String>) {
    if h24 {
        (now.format("%H:%M").to_string(), None)
    } else {
        (now.format("%-I:%M").to_string(), Some(now.format("%p").to_string()))
    }
}

fn date_text(now: DateTime<Local>, h24: bool) -> String {
    if h24 { now.format("%a %-d %b") } else { now.format("%a, %b %-d") }.to_string().to_uppercase()
}

/// Sizes of the digits and the AM/PM suffix, and the gap between them (all scaled).
fn time_metrics(fonts: &Fonts, look: &Look, size: f32, time: &str, suffix: Option<&str>) -> (f32, f32, f32, f32, f32) {
    let s = size * look.text_scale.clamp(60, 140) as f32 / 100.0;
    let ss = s * 0.3;
    let wt = text_width(&fonts.bold, s, time);
    let ws = suffix.map_or(0.0, |x| text_width(&fonts.regular, ss, x));
    let gap = if suffix.is_some() { s * 0.07 } else { 0.0 };
    (s, ss, wt, ws, gap)
}

/// Time centred at `cx`, with a smaller AM/PM top-aligned to the right of the digits.
fn push_time(t: &mut Texts, fonts: &Fonts, size: f32, cx: f32, baseline: f32, now: DateTime<Local>, h24: bool, color: [u8; 4]) {
    let (time, suffix) = time_parts(now, h24);
    let (s, ss, wt, ws, gap) = time_metrics(fonts, t.look, size, &time, suffix.as_deref());
    let left = cx - (wt + gap + ws) / 2.0;
    t.items.push(Text { bold: true, size: s, cx: left + wt / 2.0, baseline, text: time, color });
    if let Some(suffix) = suffix {
        // Cap heights are ~0.7 of the size: line up the tops of digits and suffix.
        let sb = baseline - 0.7 * s + 0.7 * ss;
        t.items.push(Text { bold: false, size: ss, cx: left + wt + gap + ws / 2.0, baseline: sb, text: suffix, color });
    }
}

fn clock(pm: &mut Pixmap, o: &ClockOptions, v: &StatValues, now: DateTime<Local>, t: &mut Texts, fonts: &Fonts) {
    let look = &o.look;
    if o.seconds_ring {
        stroke_arc(pm, 226.0, -90.0, 270.0, 4.0, look.accent, 120);
        let secs = now.format("%S").to_string().parse::<f32>().unwrap_or(0.0);
        stroke_arc(pm, 226.0, -90.0, -90.0 + 360.0 * secs / 60.0, 4.0, look.accent, 255);
    }
    let has_stats = o.stats != ClockStats::None;
    let time_y = if o.date || has_stats { 250.0 } else { 285.0 };
    push_time(t, fonts, 132.0, CENTER, time_y, now, o.h24, rgba(look.text, 255));
    if o.date {
        t.add(false, 34.0, CENTER, 300.0, date_text(now, o.h24), rgba(look.accent, 255));
    }
    let pair = match o.stats {
        ClockStats::Temps => Some((Stat::CpuTemp, Stat::GpuTemp)),
        ClockStats::Loads => Some((Stat::CpuLoad, Stat::GpuLoad)),
        ClockStats::None => None,
    };
    if let Some((a, b)) = pair {
        let y = if o.date { 366.0 } else { 330.0 };
        for (stat, cx) in [(a, CENTER - 72.0), (b, CENTER + 72.0)] {
            let value = v.get(stat);
            if let Some(dot) = PathBuilder::from_circle(cx - 58.0, y, 6.0) {
                pm.fill_path(&dot, &paint(stat_color(look, stat, value), 255), FillRule::Winding, Transform::identity(), None);
            }
            t.add(false, 32.0, cx + 4.0, y + 12.0, format!("{} {}", stat.label(), format_value(stat, value)), rgba(look.text, 230));
        }
    }
}

// ---------------------------------------------------------------------------------------
// Custom designs

fn widget_color(look: &Look, w: &Widget, v: &StatValues) -> Rgb {
    match (w.auto_color, w.kind) {
        (false, _) => w.color,
        (true, WidgetKind::Ring | WidgetKind::Bar) => {
            if w.sensor_key.is_some() { look.accent } else { stat_color(look, w.stat, v.get(w.stat)) }
        },
        (true, _) => look.text,
    }
}

/// The text a text widget shows (time widgets also return their AM/PM suffix).
fn widget_text(d: &CustomDesign, w: &Widget, v: &StatValues, now: DateTime<Local>) -> (String, Option<String>) {
    match w.kind {
        WidgetKind::Value => (v.widget_text(w), None),
        WidgetKind::Label => (w.text.clone(), None),
        WidgetKind::Time => time_parts(now, d.h24),
        WidgetKind::Date => (date_text(now, d.h24), None),
        WidgetKind::Ring | WidgetKind::Bar => (String::new(), None),
    }
}

fn text_baseline(y: f32, size: f32) -> f32 {
    // Centre the cap height (~0.7 of the size) on y.
    y + size * 0.35
}

fn custom(pm: &mut Pixmap, d: &CustomDesign, v: &StatValues, now: DateTime<Local>, t: &mut Texts, fonts: &Fonts) {
    let track = [255, 255, 255];
    for w in &d.widgets {
        let color = widget_color(&d.look, w, v);
        let scaled = w.size * d.look.text_scale.clamp(60, 140) as f32 / 100.0;
        match w.kind {
            WidgetKind::Ring => {
                let (start, sweep) = (135.0_f32, 270.0_f32);
                stroke_arc_at(pm, w.x, w.y, w.size, start, start + sweep, w.thickness, track, 34);
                stroke_arc_at(pm, w.x, w.y, w.size, start, start + sweep * v.widget_fraction(w), w.thickness, color, 255);
            }
            WidgetKind::Bar => {
                let (x, y, h) = (w.x - w.size / 2.0, w.y - w.thickness / 2.0, w.thickness.max(2.0));
                fill_rounded_rect(pm, x, y, w.size, h, h / 2.0, track, 40);
                let filled = (w.size * v.widget_fraction(w)).max(h);
                fill_rounded_rect(pm, x, y, filled, h, h / 2.0, color, 255);
            }
            WidgetKind::Time => push_time(t, fonts, w.size, w.x, text_baseline(w.y, scaled), now, d.h24, rgba(color, 255)),
            _ => {
                let (text, _) = widget_text(d, w, v, now);
                t.items.push(Text { bold: w.bold, size: scaled, cx: w.x, baseline: text_baseline(w.y, scaled), text, color: rgba(color, 255) });
            }
        }
    }
}

/// Bounding box (x0, y0, x1, y1) of widget `i` of a custom design, for the editor.
pub fn widget_bounds(d: &CustomDesign, i: usize, v: &StatValues, now: DateTime<Local>) -> Option<[f32; 4]> {
    let w = d.widgets.get(i)?;
    let fonts = fonts(d.look.font)?;
    let scaled = w.size * d.look.text_scale.clamp(60, 140) as f32 / 100.0;
    let (hw, hh) = match w.kind {
        WidgetKind::Ring => (w.size + w.thickness / 2.0, w.size + w.thickness / 2.0),
        WidgetKind::Bar => (w.size / 2.0, w.thickness.max(2.0) / 2.0),
        WidgetKind::Time => {
            let (time, suffix) = time_parts(now, d.h24);
            let (_, _, wt, ws, gap) = time_metrics(fonts, &d.look, w.size, &time, suffix.as_deref());
            ((wt + gap + ws) / 2.0, scaled * 0.4)
        }
        _ => {
            let (text, _) = widget_text(d, w, v, now);
            let font = if w.bold { &fonts.bold } else { &fonts.regular };
            (text_width(font, scaled, &text).max(8.0) / 2.0, scaled * 0.4)
        }
    };
    Some([w.x - hw, w.y - hh, w.x + hw, w.y + hh])
}

/// The topmost widget at (x, y) on the 480x480 canvas.
pub fn hit_test(d: &CustomDesign, v: &StatValues, now: DateTime<Local>, x: f32, y: f32) -> Option<usize> {
    (0..d.widgets.len()).rev().find(|&i| {
        let w = &d.widgets[i];
        if w.kind == WidgetKind::Ring {
            // Only the ring itself, so widgets inside it stay clickable.
            let dist = ((x - w.x).powi(2) + (y - w.y).powi(2)).sqrt();
            (dist - w.size).abs() <= w.thickness / 2.0 + 10.0
        } else {
            widget_bounds(d, i, v, now).is_some_and(|[x0, y0, x1, y1]| x >= x0 - 8.0 && x <= x1 + 8.0 && y >= y0 - 8.0 && y <= y1 + 8.0)
        }
    })
}

/// Render the overlay. Returns a fully transparent image when stats are disabled.
pub fn render(cfg: &StatsConfig, values: &StatValues, now: DateTime<Local>) -> RgbaImage {
    let blank = || RgbaImage::new(SCREEN_SIZE, SCREEN_SIZE);
    let look = cfg.look();
    let (Some(fonts), Some(mut pm)) = (fonts(look.font), Pixmap::new(SCREEN_SIZE, SCREEN_SIZE)) else { return blank() };
    if !cfg.enabled {
        return blank();
    }
    scrim(&mut pm, look.dim);
    let mut texts = Texts { look, items: Vec::new() };
    match cfg.design {
        Design::Gauges => gauges(&mut pm, &cfg.gauges, values, &mut texts),
        Design::Tiles => tiles(&mut pm, &cfg.tiles, values, &mut texts, fonts),
        Design::BigStat => big_stat(&mut pm, &cfg.big_stat, values, &mut texts),
        Design::Clock => clock(&mut pm, &cfg.clock, values, now, &mut texts, fonts),
        Design::Custom(_) => {
            if let Some(d) = cfg.custom_design() {
                custom(&mut pm, d, values, now, &mut texts, fonts);
            }
        }
    }

    // tiny-skia stores premultiplied RGBA; convert to straight alpha for text blending and PNG.
    let mut img = blank();
    for (dst, src) in img.pixels_mut().zip(pm.pixels()) {
        let c = src.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    for t in texts.items {
        let font = if t.bold { &fonts.bold } else { &fonts.regular };
        let x = t.cx - text_width(font, t.size, &t.text) / 2.0;
        if look.outline {
            // A dark halo: the text stamped in black around a small circle, then the text.
            let r = (t.size * 0.045).clamp(1.5, 4.0);
            for i in 0..12 {
                let a = i as f32 * PI / 6.0;
                draw_text(&mut img, font, t.size, x + r * a.cos(), t.baseline + r * a.sin(), &t.text, [0, 0, 0, 200]);
            }
        }
        draw_text(&mut img, font, t.size, x, t.baseline, &t.text, t.color);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_and_font_renders_and_disabled_is_blank() {
        let mut cfg = StatsConfig::default();
        let now = Local::now();
        for design in Design::ALL {
            cfg.design = design;
            for font in FontChoice::ALL {
                cfg.look_mut().font = font;
                cfg.look_mut().outline = font == FontChoice::Bahnschrift;
                let img = render(&cfg, &StatValues::sample(), now);
                assert!(img.pixels().any(|p| p[3] == 255), "{design:?}/{font:?} drew nothing opaque");
                if let Ok(dir) = std::env::var("CARGO_TARGET_TMPDIR") {
                    let _ = img.save(std::path::Path::new(&dir).join(format!("design-{design:?}-{font:?}.png")));
                }
            }
        }
        cfg.enabled = false;
        assert!(render(&cfg, &StatValues::sample(), now).pixels().all(|p| p[3] == 0));
    }

    #[test]
    fn custom_design_renders_and_hit_tests() {
        let mut cfg = StatsConfig::default();
        cfg.custom.push(CustomDesign::starter(7, "Mine".into()));
        cfg.design = Design::Custom(7);
        let (v, now) = (StatValues::sample(), Local::now());
        let img = render(&cfg, &v, now);
        assert!(img.pixels().any(|p| p[3] == 255));
        if let Ok(dir) = std::env::var("CARGO_TARGET_TMPDIR") {
            let _ = img.save(std::path::Path::new(&dir).join("design-Custom.png"));
        }
        let d = cfg.custom_design().unwrap();
        // The big value in the middle, not the ring around it.
        assert_eq!(hit_test(d, &v, now, 240.0, 245.0), Some(2));
        // The ring at its top edge (radius 200 around the centre).
        assert_eq!(hit_test(d, &v, now, 240.0, 40.0), Some(0));
        assert_eq!(hit_test(d, &v, now, 20.0, 470.0), None);
    }

    #[test]
    fn twelve_hour_time_has_am_pm() {
        let (time, suffix) = time_parts(Local::now(), false);
        assert!(!time.is_empty());
        assert!(matches!(suffix.as_deref(), Some("AM") | Some("PM")));
        assert!(time_parts(Local::now(), true).1.is_none());
        let mut cfg = StatsConfig { design: Design::Clock, ..Default::default() };
        cfg.clock.h24 = false;
        let img = render(&cfg, &StatValues::sample(), Local::now());
        if let Ok(dir) = std::env::var("CARGO_TARGET_TMPDIR") {
            let _ = img.save(std::path::Path::new(&dir).join("design-Clock12h.png"));
        }
    }

    #[test]
    fn temperature_colours_run_cool_to_hot() {
        assert_eq!(temp_color(20.0), [56, 189, 248]);
        assert_eq!(temp_color(95.0), [244, 63, 94]);
    }
}
