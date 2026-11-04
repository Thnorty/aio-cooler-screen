//! Windows-style GUI (Slint, Fluent style). Runs without admin: edits the shared config,
//! prepares media into the data dir, and shows what the service reports.

slint::include_modules!();

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel};
use windows_service::service::ServiceState;

use crate::config::{self, ClockStats, Config, CustomDesign, Design, FontChoice, Position, StatValues, Status, Widget, WidgetKind};
use crate::media::{self, Fit, IMAGE_EXTENSIONS, SCREEN_SIZE, VIDEO_EXTENSIONS};
use crate::sensors::{Reading, Stat};
use crate::desktop::{self, Instance, Prefs, Tray, TrayAction};
use crate::{ffmpeg, icon, overlay, service};

const STATUS_STALE_MS: u128 = 5_000;
const SWATCHES: [[u8; 3]; 8] = [
    [255, 255, 255],
    [203, 213, 225],
    [56, 189, 248],
    [74, 222, 128],
    [251, 191, 36],
    [244, 114, 182],
    [244, 63, 94],
    [167, 139, 250],
];
const FITS: [Fit; 3] = [Fit::Cover, Fit::Contain, Fit::Stretch];
const POSITIONS: [Position; 3] = [Position::Top, Position::Center, Position::Bottom];
const CLOCK_STATS: [ClockStats; 3] = [ClockStats::Temps, ClockStats::Loads, ClockStats::None];
/// Displayed size of the LCD preview, in logical pixels (the canvas is 480).
const PREVIEW_PX: f32 = 300.0;

/// Backgrounds shipped with the app (made with tools/make-backgrounds.ps1).
const BACKGROUNDS: [(&str, &[u8]); 4] = [
    ("Nebula", include_bytes!("../assets/backgrounds/nebula.png")),
    ("Aurora", include_bytes!("../assets/backgrounds/aurora.png")),
    ("Neon rings", include_bytes!("../assets/backgrounds/neon-rings.png")),
    ("Sunset", include_bytes!("../assets/backgrounds/sunset.png")),
];
const BUILTIN_SUFFIX: &str = " (built-in)";

enum Job {
    Poster(Result<RgbaImage, String>),
    Media(Result<(String, bool, RgbaImage), String>, String),
    ServiceRestarted(Result<(), String>),
    Autostart(Result<(), String>),
    FfmpegFound(Option<PathBuf>),
    FfmpegDownloaded(Result<(), String>),
}

struct State {
    cfg: Config,
    readings: Vec<Reading>,
    live: bool,
    preview_bg: RgbaImage,
    jobs: (mpsc::Sender<Job>, mpsc::Receiver<Job>),
    last_poll: Instant,
    last_process_check: Instant,
    last_fit: i32,
    rows: Rc<VecModel<StatRow>>,
    row_keys: Vec<Vec<String>>,
    rows_sig: String,
    ffmpeg: Option<PathBuf>,
    ffmpeg_progress_file: Option<PathBuf>,
    /// Custom design editor: the selected widget and, while dragging, the grab offset.
    selected_widget: Option<usize>,
    drag_offset: Option<(f32, f32)>,
    prefs: Prefs,
    tray: Option<Tray>,
    last_theme_check: Instant,
}

impl State {
    fn spawn(&self, f: impl FnOnce() -> Job + Send + 'static) {
        let tx = self.jobs.0.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
    }

    fn values(&self) -> StatValues {
        if self.live {
            self.cfg.stats.resolve(&self.readings)
        } else {
            StatValues::sample()
        }
    }
}

/// Run the GUI. With `start_in_tray` (used by the sign-in entry) only the tray icon appears.
pub fn run(start_in_tray: bool) -> Result<(), String> {
    // One copy at a time: a second launch just brings the running one forward.
    let _instance = match desktop::claim_instance() {
        Instance::First(handle) => handle,
        Instance::AlreadyRunning => return Ok(()),
    };
    // Skia renders text and shapes with proper anti-aliasing; fall back if it can't start.
    if slint::BackendSelector::new().renderer_name("skia".into()).select().is_err() {
        let _ = slint::BackendSelector::new().select();
    }
    let ui = AppWindow::new().map_err(|e| e.to_string())?;
    let prefs = Prefs::load();
    let state = Rc::new(RefCell::new(State {
        prefs,
        tray: None,
        last_theme_check: Instant::now(),
        preview_bg: default_background(),
        cfg: Config::load(),
        readings: Vec::new(),
        live: false,
        jobs: mpsc::channel(),
        last_poll: Instant::now() - Duration::from_secs(10),
        last_process_check: Instant::now() - Duration::from_secs(10),
        last_fit: 0,
        rows: Rc::new(VecModel::default()),
        row_keys: Vec::new(),
        rows_sig: String::new(),
        ffmpeg: None,
        ffmpeg_progress_file: None,
        selected_widget: None,
        drag_offset: None,
    }));

    init_ui(&ui, &state.borrow());
    {
        let mut s = state.borrow_mut();
        load_current_poster(&s);
        s.spawn(|| Job::FfmpegFound(ffmpeg::find()));
        let show = s.prefs.show_tray;
        set_tray(&ui, &mut s, show);
    }
    wire_callbacks(&ui, &state);

    // Closing the window keeps the app in the tray while the tray icon is shown.
    let st = state.clone();
    ui.window().on_close_requested(move || {
        if st.borrow().tray.is_none() {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::HideWindow
    });
    let weak = ui.as_weak();
    desktop::listen_for_show(move || {
        let _ = weak.upgrade_in_event_loop(show_window);
    });

    let timer = Timer::default();
    let (weak, st) = (ui.as_weak(), state.clone());
    timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let Some(ui) = weak.upgrade() {
            tick(&ui, &st);
        }
    });
    if !start_in_tray || state.borrow().tray.is_none() {
        ui.show().map_err(|e| e.to_string())?;
    }
    slint::run_event_loop_until_quit().map_err(|e| e.to_string())
}

fn show_window(ui: AppWindow) {
    let _ = ui.show();
    ui.window().set_minimized(false);
}

/// Create or remove the tray icon, and reflect it in the Settings controls.
fn set_tray(ui: &AppWindow, s: &mut State, on: bool) {
    if on && s.tray.is_none() {
        match Tray::new(s.cfg.stats.enabled, s.cfg.display_off, s.prefs.language) {
            Ok(t) => s.tray = Some(t),
            Err(e) => show_message(ui, false, format!("{}: {e}", if s.prefs.language == 1 { "Sistem tepsisi simgesi oluşturulamadı" } else { "Could not create the tray icon" })),
        }
    } else if !on {
        s.tray = None;
    }
    ui.set_show_tray(s.tray.is_some());
    ui.set_login_tray(desktop::open_at_login());
}

/// Turn the cooler screen off or back on (the service acts on the saved config).
fn set_display_off(ui: &AppWindow, s: &mut State, off: bool) {
    s.cfg.display_off = off;
    save(ui, s);
    ui.set_display_off(off);
    if let Some(t) = &s.tray {
        t.set_display_off(off);
    }
}

// ---------------------------------------------------------------------------------------
// Images

fn to_image(img: &RgbaImage) -> Image {
    Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

/// Shown when no background media is set: a deep blue gradient.
fn default_background() -> RgbaImage {
    RgbaImage::from_fn(SCREEN_SIZE, SCREEN_SIZE, |x, y| {
        let t = (x + y) as f32 / (2 * SCREEN_SIZE) as f32;
        let mix = |a: f32, b: f32| (a + (b - a) * t) as u8;
        Rgba([mix(30.0, 8.0), mix(41.0, 12.0), mix(75.0, 28.0), 255])
    })
}

fn compose(bg: &RgbaImage, cfg: &config::StatsConfig, values: &StatValues) -> RgbaImage {
    let mut img = bg.clone();
    image::imageops::overlay(&mut img, &overlay::render(cfg, values, chrono::Local::now()), 0, 0);
    img
}

/// Every design in gallery order: the built-ins, then the user's own.
fn gallery(cfg: &Config) -> Vec<Design> {
    Design::ALL.into_iter().chain(cfg.stats.custom.iter().map(|d| Design::Custom(d.id))).collect()
}

fn design_index(cfg: &Config) -> i32 {
    gallery(cfg).iter().position(|d| *d == cfg.stats.design).unwrap_or(0) as i32
}

/// Rebuild the gallery: thumbnails and names for every design.
fn refresh_gallery(ui: &AppWindow, state: &State) {
    let bg = default_background();
    let designs = gallery(&state.cfg);
    let thumbs: Vec<Image> = designs
        .iter()
        .map(|&design| {
            let mut stats = state.cfg.stats.clone();
            stats.enabled = true;
            stats.design = design;
            let img = compose(&bg, &stats, &StatValues::sample());
            to_image(&image::imageops::resize(&img, 224, 224, FilterType::Lanczos3))
        })
        .collect();
    let names: Vec<SharedString> = designs
        .iter()
        .map(|d| match d {
            Design::Custom(id) => state.cfg.stats.custom.iter().find(|c| c.id == *id).map_or((if state.prefs.language == 1 { "Özel" } else { "Custom" }).into(), |c| c.name.as_str().into()),
            d => (if state.prefs.language == 1 { match d { Design::Gauges => "Halka göstergeleri", Design::Tiles => "Gösterge panosu", Design::BigStat => "Büyük istatistik", Design::Clock => "Saat ve istatistikler", Design::Custom(_) => "Özel" } } else { d.name() }).into(),
        })
        .collect();
    ui.set_design_names(ModelRc::new(VecModel::from(names)));
    ui.set_design_thumbs(ModelRc::new(VecModel::from(thumbs)));
    ui.set_design(design_index(&state.cfg));
}

fn refresh_preview(ui: &AppWindow, state: &State) {
    let mut img = compose(&state.preview_bg, &state.cfg.stats, &state.values());
    if let (Some(d), Some(i)) = (state.cfg.stats.custom_design(), state.selected_widget) {
        if let Some(b) = overlay::widget_bounds(d, i, &state.values(), chrono::Local::now()) {
            outline(&mut img, b);
        }
    }
    ui.set_preview(to_image(&img));
    ui.set_preview_live(state.live);
}

/// Dashed selection rectangle, drawn only in the preview.
fn outline(img: &mut RgbaImage, [x0, y0, x1, y1]: [f32; 4]) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (x0, y0, x1, y1) = ((x0 - 6.0) as i32, (y0 - 6.0) as i32, (x1 + 6.0) as i32, (y1 + 6.0) as i32);
    let mut put = |x: i32, y: i32, along: i32| {
        if x >= 0 && y >= 0 && x < w && y < h {
            let c = if (along / 6) % 2 == 0 { Rgba([255, 255, 255, 255]) } else { Rgba([20, 20, 20, 255]) };
            img.put_pixel(x as u32, y as u32, c);
        }
    };
    for t in 0..2 {
        for x in x0..=x1 {
            put(x, y0 - t, x);
            put(x, y1 + t, x);
        }
        for y in y0..=y1 {
            put(x0 - t, y, y);
            put(x1 + t, y, y);
        }
    }
}

fn load_current_poster(state: &State) {
    let Some(path) = state.cfg.media_file.as_ref().map(|f| config::data_dir().join(f)).filter(|p| p.exists()) else {
        return;
    };
    let is_video = state.cfg.media_is_video;
    state.spawn(move || {
        Job::Poster(if is_video { media::video_poster(&path, Fit::Cover) } else { media::prepare_image(&path, Fit::Cover) })
    });
}

fn poster_for(path: &Path, fit: Fit) -> Result<RgbaImage, String> {
    if media::is_video(path) {
        media::video_poster(path, fit)
    } else {
        media::prepare_image(path, fit)
    }
}

// ---------------------------------------------------------------------------------------
// Config <-> UI

fn swatch_index(rgb: [u8; 3]) -> i32 {
    SWATCHES.iter().position(|s| *s == rgb).map_or(-1, |i| i as i32)
}

fn swatch(index: i32) -> Option<[u8; 3]> {
    usize::try_from(index).ok().and_then(|i| SWATCHES.get(i)).copied()
}

fn index_of<T: PartialEq>(all: &[T], v: &T) -> i32 {
    all.iter().position(|x| x == v).unwrap_or(0) as i32
}

/// Which built-in background is on the screen, or -1.
fn builtin_index(cfg: &Config) -> i32 {
    let Some(src) = cfg.media_source.as_deref().filter(|_| !cfg.media_is_video) else { return -1 };
    BACKGROUNDS.iter().position(|(n, _)| format!("{n}{BUILTIN_SUFFIX}") == src).map_or(-1, |i| i as i32)
}

fn media_label(cfg: &Config, lang: i32) -> String {
    match (&cfg.media_source, &cfg.media_file) {
        (Some(src), _) => format!("{src}{}", if cfg.media_is_video { " (video)" } else { "" }),
        (None, Some(_)) => (if cfg.media_is_video { if lang == 1 { "Videon" } else { "Your video" } } else if lang == 1 { "Görselin" } else { "Your image" }).into(),
        (None, None) => (if lang == 1 { "Arka plan ayarlanmadı" } else { "No background set" }).into(),
    }
}

fn init_ui(ui: &AppWindow, state: &State) {
    ui.set_app_icon(to_image(&icon::render(64)));
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_language(state.prefs.language.clamp(0, 1));
    ui.set_preview_shape(state.prefs.preview_shape.clamp(0, 1));
    ui.set_display_off(state.cfg.display_off);
    let kinds: Vec<SharedString> = WidgetKind::ALL.iter().map(|k| {
        let name = localized_widget_kind(*k, state.prefs.language); name.into()
    }).collect();
    ui.set_widget_kinds(ModelRc::new(VecModel::from(kinds)));
    let bg_thumbs: Vec<Image> = BACKGROUNDS
        .iter()
        .filter_map(|(_, png)| image::load_from_memory(png).ok())
        .map(|img| to_image(&img.resize_exact(184, 184, FilterType::Lanczos3).to_rgba8()))
        .collect();
    ui.set_bg_thumbs(ModelRc::new(VecModel::from(bg_thumbs)));
    let bg_names: Vec<SharedString> = BACKGROUNDS.iter().map(|(n, _)| SharedString::from(*n)).collect();
    ui.set_bg_names(ModelRc::new(VecModel::from(bg_names)));
    ui.set_bg_selected(builtin_index(&state.cfg));
    refresh_gallery(ui, state);
    let swatches: Vec<slint::Color> = SWATCHES.iter().map(|c| slint::Color::from_rgb_u8(c[0], c[1], c[2])).collect();
    ui.set_swatches(ModelRc::new(VecModel::from(swatches)));
    ui.set_stat_rows(ModelRc::from(state.rows.clone()));
    ui.set_media_name(media_label(&state.cfg, state.prefs.language).into());
    ui.set_lhm_port(state.cfg.effective_lhm_port().to_string().into());
    ui.set_autostart(service::query_autostart().unwrap_or(false));
    ui.set_ffmpeg_state((if state.prefs.language == 1 { "Kontrol ediliyor…" } else { "Checking…" }).into());
    load_design_into_ui(ui, &state.cfg);
    refresh_preview(ui, state);
}

/// Show the selected design's look and every design's options in the controls.
fn load_design_into_ui(ui: &AppWindow, cfg: &Config) {
    let s = &cfg.stats;
    ui.set_design(design_index(cfg));
    ui.set_stats_enabled(s.enabled);
    if let Some(d) = s.custom_design() {
        ui.set_custom_name(d.name.as_str().into());
        ui.set_cu_h24(d.h24);
    }

    let look = s.look();
    ui.set_text_color_index(swatch_index(look.text));
    ui.set_accent_index(swatch_index(look.accent));
    ui.set_temp_colors(look.temp_colors);
    ui.set_dim(look.dim as f32);
    ui.set_font_index(index_of(&FontChoice::ALL, &look.font));
    ui.set_text_scale(look.text_scale as f32);
    ui.set_outline(look.outline);

    let g = &s.gauges;
    ui.set_g_load_rings(g.load_rings);
    ui.set_g_labels(g.labels);
    ui.set_g_thickness(g.thickness as f32);

    let t = &s.tiles;
    ui.set_t_cpu_temp(t.show.contains(&Stat::CpuTemp));
    ui.set_t_cpu_load(t.show.contains(&Stat::CpuLoad));
    ui.set_t_gpu_temp(t.show.contains(&Stat::GpuTemp));
    ui.set_t_gpu_load(t.show.contains(&Stat::GpuLoad));
    ui.set_t_columns(if t.columns >= 2 { 1 } else { 0 });
    ui.set_t_position(index_of(&POSITIONS, &t.position));
    ui.set_t_panel(t.panel);
    ui.set_t_panel_opacity(t.panel_opacity as f32);

    let b = &s.big_stat;
    ui.set_b_stat(index_of(&Stat::ALL, &b.stat));
    ui.set_b_ring(b.ring);
    ui.set_b_ticks(b.ticks);
    ui.set_b_secondary(b.secondary);

    let c = &s.clock;
    ui.set_c_h24(c.h24);
    ui.set_c_seconds(c.seconds_ring);
    ui.set_c_date(c.date);
    ui.set_c_stats(index_of(&CLOCK_STATS, &c.stats));
}

/// Which widget properties the editor shows for each kind, and the size slider's range.
fn size_range(kind: WidgetKind, lang: i32) -> (&'static str, f32, f32) {
    match kind {
        WidgetKind::Ring => (if lang == 1 { "Yarıçap" } else { "Radius" }, 20.0, 235.0),
        WidgetKind::Bar => (if lang == 1 { "Genişlik" } else { "Width" }, 20.0, 460.0),
        _ => (if lang == 1 { "Metin boyutu" } else { "Text size" }, 10.0, 220.0),
    }
}

fn localized_widget_kind(kind: WidgetKind, lang: i32) -> &'static str {
    if lang != 1 { return kind.name(); }
    match kind {
        WidgetKind::Value => "İstatistik değeri",
        WidgetKind::Label => "Metin",
        WidgetKind::Ring => "Halka göstergesi",
        WidgetKind::Bar => "İlerleme çubuğu",
        WidgetKind::Time => "Saat",
        WidgetKind::Date => "Tarih",
    }
}

fn widget_label(w: &Widget, lang: i32, readings: &[Reading]) -> String {
    match w.kind {
        WidgetKind::Label => format!("{} \"{}\"", localized_widget_kind(w.kind, lang), w.text),
        k if k.uses_stat() => {
            let name = match w.sensor_key.as_deref() {
                Some(key) => readings.iter()
                    .find(|r| r.key == key)
                    .map(friendly_sensor_label)
                    .unwrap_or_else(|| {
                        // The selected reading may be temporarily unavailable.
                        // Keep its stable key intact and make the offline state explicit.
                        let status = if lang == 1 { "Çevrimdışı" } else { "Offline" };
                        format!("{status}: {key}")
                    }),
                None => w.stat.title().to_string(),
            };
            format!("{}: {}", localized_widget_kind(k, lang), name)
        }
        k => localized_widget_kind(k, lang).to_string(),
    }
}

/// Fill the editor: widget list and the selected widget's properties.
fn load_widgets_into_ui(ui: &AppWindow, state: &State) {
    let Some(d) = state.cfg.stats.custom_design() else {
        ui.set_widget_names(ModelRc::new(VecModel::<SharedString>::default()));
        ui.set_selected_widget(-1);
        return;
    };
    let names: Vec<SharedString> = d.widgets.iter().map(|w| widget_label(w, state.prefs.language, &state.readings).into()).collect();
    ui.set_widget_names(ModelRc::new(VecModel::from(names)));
    let Some((i, w)) = state.selected_widget.and_then(|i| d.widgets.get(i).map(|w| (i, w))) else {
        ui.set_selected_widget(-1);
        return;
    };
    ui.set_selected_widget(i as i32);
    ui.set_w_kind_name(localized_widget_kind(w.kind, state.prefs.language).into());
    ui.set_w_uses_stat(w.kind.uses_stat());
    ui.set_w_is_text(w.kind.is_text());
    ui.set_w_is_label(w.kind == WidgetKind::Label);
    ui.set_w_has_thickness(matches!(w.kind, WidgetKind::Ring | WidgetKind::Bar));
    let (label, min, max) = size_range(w.kind, state.prefs.language);
    ui.set_w_size_label(label.into());
    ui.set_w_size_min(min);
    ui.set_w_size_max(max);
    let mut labels: Vec<SharedString> = Stat::ALL.iter().map(|v| v.title().into()).collect();
    let mut keys: Vec<Option<String>> = vec![None; Stat::ALL.len()];
    // Include any currently saved sensor even when its source is temporarily offline.
    let readings = sorted_sensor_readings(&state.readings);
    for r in readings {
        labels.push(friendly_sensor_label(r).into());
        keys.push(Some(r.key.clone()));
    }
    if let Some(key) = &w.sensor_key {
        if !keys.iter().any(|k| k.as_ref() == Some(key)) {
            labels.push(format!("{}: {key}", if state.prefs.language == 1 { "Çevrimdışı" } else { "Offline" }).into());
            keys.push(Some(key.clone()));
        }
    }
    let selected = w.sensor_key.as_ref().and_then(|key| keys.iter().position(|k| k.as_ref() == Some(key)))
        .unwrap_or_else(|| index_of(&Stat::ALL, &w.stat) as usize);
    ui.set_widget_sensor_choices(ModelRc::new(VecModel::from(labels)));
    ui.set_w_stat(selected as i32);
    ui.set_w_sensor_max(w.sensor_max as f32);
    ui.set_w_text(w.text.as_str().into());
    ui.set_w_size(w.size);
    ui.set_w_thickness(w.thickness);
    ui.set_w_x(w.x);
    ui.set_w_y(w.y);
    ui.set_w_bold(w.bold);
    ui.set_w_auto(w.auto_color);
    ui.set_w_color_index(swatch_index(w.color));
}

// Friendly, category-first sensor labels. Keep Reading::key unchanged so saved designs
// continue to resolve the same hardware readings after this display-only change.
fn sorted_sensor_readings(readings: &[Reading]) -> Vec<&Reading> {
    let mut result: Vec<&Reading> = readings.iter().filter(|r| r.kind > 0).collect();
    result.sort_by(|a, b| {
        sensor_category(a.kind).0.cmp(&sensor_category(b.kind).0)
            .then(a.sensor.cmp(&b.sensor))
            .then(a.name.cmp(&b.name))
            .then(a.key.cmp(&b.key))
    });
    result
}

fn sensor_category(kind: i32) -> (u8, &'static str) {
    match kind {
        3 => (0, "Fan / pompa (RPM)"),
        1 => (1, "Sicaklik (°C)"),
        7 => (2, "Kullanim (%)"),
        5 => (3, "Guc (W)"),
        2 => (4, "Voltaj (V)"),
        6 => (5, "Frekans (MHz)"),
        4 => (6, "Akim (A)"),
        _ => (7, "Diger"),
    }
}

fn friendly_sensor_label(r: &Reading) -> String {
    // Source prefix stays visible to distinguish identically named sensors.
    // The complete original reading key is preserved separately, never translated.
    let source = if r.sensor.starts_with("Internal LHM · ") { "Dahili LHM" }
        else if r.sensor.starts_with("LHM · ") { "LHM" }
        else if r.sensor.starts_with("HWiNFO · ") { "HWiNFO" }
        else if r.sensor.starts_with("NVML · ") { "NVML" }
        else { "Sensor" };
    let hardware = r.sensor.split_once(" · ").map(|(_, v)| v).unwrap_or(&r.sensor);
    let unit = if r.units.is_empty() { "" } else { r.units.as_str() };
    let category = sensor_category(r.kind).1;
    if unit.is_empty() {
        format!("{category} | {} — {hardware} [{source}]", r.name)
    } else {
        format!("{category} | {} — {hardware} [{source}, {unit}]", r.name)
    }
}

/// Copy the editor's property controls into the selected widget.
fn read_widget(ui: &AppWindow, state: &mut State) {
    let Some(i) = state.selected_widget else { return };
    let readings = sorted_sensor_readings(&state.readings);
    let Some(w) = state.cfg.stats.custom_design_mut().and_then(|d| d.widgets.get_mut(i)) else { return };
    let choice = ui.get_w_stat().max(0) as usize;
    if choice < Stat::ALL.len() {
        w.stat = Stat::ALL[choice];
        w.sensor_key = None;
    } else {
        // This list is rebuilt in load_widgets_into_ui, from the same reading order.
        if let Some(r) = readings.get(choice - Stat::ALL.len()) {
            w.sensor_key = Some(r.key.clone());
        }
    }
    w.sensor_max = (ui.get_w_sensor_max() as f64).clamp(1.0, 100000.0);
    w.text = ui.get_w_text().to_string();
    w.size = ui.get_w_size().round();
    w.thickness = ui.get_w_thickness().round();
    w.x = ui.get_w_x().round();
    w.y = ui.get_w_y().round();
    w.bold = ui.get_w_bold();
    w.auto_color = ui.get_w_auto();
    if let Some(c) = swatch(ui.get_w_color_index()) {
        w.color = c;
    }
}

/// Map a point on the displayed preview to the 480x480 canvas.
fn to_canvas(x: f32, y: f32) -> (f32, f32) {
    let k = SCREEN_SIZE as f32 / PREVIEW_PX;
    (x * k, y * k)
}

/// Copy the controls back into the config (the look goes to the selected design).
fn read_settings(ui: &AppWindow, cfg: &mut Config) {
    let s = &mut cfg.stats;
    s.enabled = ui.get_stats_enabled();
    if let Some(d) = s.custom_design_mut() {
        d.name = ui.get_custom_name().trim().to_string();
        if d.name.is_empty() {
            d.name = "Tasarımım".into();
        }
        d.h24 = ui.get_cu_h24();
    }

    let look = s.look_mut();
    if let Some(c) = swatch(ui.get_text_color_index()) {
        look.text = c;
    }
    if let Some(c) = swatch(ui.get_accent_index()) {
        look.accent = c;
    }
    look.temp_colors = ui.get_temp_colors();
    look.dim = ui.get_dim().round().clamp(0.0, 80.0) as u8;
    look.font = FontChoice::ALL[ui.get_font_index().clamp(0, (FontChoice::ALL.len() - 1) as i32) as usize];
    look.text_scale = ui.get_text_scale().round().clamp(70.0, 130.0) as u8;
    look.outline = ui.get_outline();

    s.gauges.load_rings = ui.get_g_load_rings();
    s.gauges.labels = ui.get_g_labels();
    s.gauges.thickness = ui.get_g_thickness().round().clamp(8.0, 36.0) as u8;

    let flags = [ui.get_t_cpu_temp(), ui.get_t_cpu_load(), ui.get_t_gpu_temp(), ui.get_t_gpu_load()];
    s.tiles.show = Stat::ALL.into_iter().zip(flags).filter(|(_, on)| *on).map(|(st, _)| st).collect();
    s.tiles.columns = if ui.get_t_columns() == 1 { 2 } else { 1 };
    s.tiles.position = POSITIONS[ui.get_t_position().clamp(0, 2) as usize];
    s.tiles.panel = ui.get_t_panel();
    s.tiles.panel_opacity = ui.get_t_panel_opacity().round().clamp(10.0, 100.0) as u8;

    s.big_stat.stat = Stat::ALL[ui.get_b_stat().clamp(0, (Stat::ALL.len() - 1) as i32) as usize];
    s.big_stat.ring = ui.get_b_ring();
    s.big_stat.ticks = ui.get_b_ticks();
    s.big_stat.secondary = ui.get_b_secondary();

    s.clock.h24 = ui.get_c_h24();
    s.clock.seconds_ring = ui.get_c_seconds();
    s.clock.date = ui.get_c_date();
    s.clock.stats = CLOCK_STATS[ui.get_c_stats().clamp(0, 2) as usize];
}

fn show_message(ui: &AppWindow, ok: bool, msg: impl Into<SharedString>) {
    ui.set_message(msg.into());
    ui.set_message_ok(ok);
}

fn save(ui: &AppWindow, state: &State) {
    if let Err(e) = state.cfg.save() {
        show_message(ui, false, format!("{}: {e}", if state.prefs.language == 1 { "Ayarlar kaydedilemedi" } else { "Could not save settings" }));
    }
}

fn update_ffmpeg_ui(ui: &AppWindow, state: &State) {
    ui.set_ffmpeg_ok(state.ffmpeg.is_some());
    ui.set_ffmpeg_state(match &state.ffmpeg {
        Some(p) if p.as_os_str() == "ffmpeg" => (if state.prefs.language == 1 { "PATH üzerinde bulundu" } else { "Found on PATH" }).into(),
        Some(p) => format!("Found: {}", p.display()).into(),
        None => (if state.prefs.language == 1 { "Kurulu değil" } else { "Not installed" }).into(),
    });
}

// ---------------------------------------------------------------------------------------
// Callbacks

fn wire_callbacks(ui: &AppWindow, state: &Rc<RefCell<State>>) {
    {
        let st = state.clone();
        ui.on_preview_shape_changed(move |shape| {
            let mut s = st.borrow_mut();
            s.prefs.preview_shape = shape.clamp(0, 1);
            if let Err(e) = s.prefs.save() { eprintln!("Failed to save preview shape: {e}"); }
        });
    }
    {
        let st = state.clone();
        let weak = ui.as_weak();
        ui.on_language_changed(move |lang| {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = st.borrow_mut();
            s.prefs.language = lang.clamp(0, 1);
            if let Err(e) = s.prefs.save() { eprintln!("Failed to save language: {e}"); }
            if let Some(tray) = &s.tray { tray.set_language(s.prefs.language); }
            // Force localized sensor-picker text to refresh immediately.
            s.rows_sig.clear();
            update_stat_rows(&mut s);
            refresh_gallery(&ui, &s);
            load_widgets_into_ui(&ui, &s);
            let kinds: Vec<SharedString> = WidgetKind::ALL.iter()
                .map(|k| localized_widget_kind(*k, s.prefs.language).into()).collect();
            ui.set_widget_kinds(ModelRc::new(VecModel::from(kinds)));
            update_ffmpeg_ui(&ui, &s);
            ui.set_media_name(media_label(&s.cfg, s.prefs.language).into());
        });
    }

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_settings_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        read_settings(&ui, &mut s.cfg);
        save(&ui, &s);
        if let Some(t) = &s.tray {
            t.set_stats_checked(s.cfg.stats.enabled);
        }
        refresh_preview(&ui, &s);
        refresh_gallery(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_design_selected(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let Some(&design) = gallery(&s.cfg).get(i.max(0) as usize) else { return };
        s.cfg.stats.design = design;
        s.cfg.stats.enabled = true;
        s.selected_widget = None;
        save(&ui, &s);
        load_design_into_ui(&ui, &s.cfg);
        load_widgets_into_ui(&ui, &s);
        refresh_preview(&ui, &s);
    });

    // --- Custom design editor ---------------------------------------------------------------
    // Applies an edit to the config, then saves and refreshes everything that shows designs.
    fn edited(ui: &AppWindow, s: &mut State) {
        save(ui, s);
        refresh_gallery(ui, s);
        load_design_into_ui(ui, &s.cfg);
        load_widgets_into_ui(ui, s);
        refresh_preview(ui, s);
    }

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_new_design(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let id = s.cfg.stats.custom.iter().map(|d| d.id).max().unwrap_or(0) + 1;
        let name = format!("{} {}", if s.prefs.language == 1 { "Tasarımım" } else { "My design" }, s.cfg.stats.custom.len() + 1);
        s.cfg.stats.custom.push(CustomDesign::starter(id, name));
        s.cfg.stats.design = Design::Custom(id);
        s.cfg.stats.enabled = true;
        s.selected_widget = Some(2);
        edited(&ui, &mut s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_duplicate_design(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let Some(mut copy) = s.cfg.stats.custom_design().cloned() else { return };
        copy.id = s.cfg.stats.custom.iter().map(|d| d.id).max().unwrap_or(0) + 1;
        copy.name = format!("{} copy", copy.name);
        s.cfg.stats.design = Design::Custom(copy.id);
        s.cfg.stats.custom.push(copy);
        edited(&ui, &mut s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_delete_design(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        if let Design::Custom(id) = s.cfg.stats.design {
            s.cfg.stats.custom.retain(|d| d.id != id);
            s.cfg.stats.design = Design::Gauges;
            s.selected_widget = None;
            edited(&ui, &mut s);
        }
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_add_widget(move |kind| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let kind = WidgetKind::ALL[kind.clamp(0, 5) as usize];
        let Some(d) = s.cfg.stats.custom_design_mut() else { return };
        d.widgets.push(Widget::new(kind));
        let i = d.widgets.len() - 1;
        s.selected_widget = Some(i);
        edited(&ui, &mut s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_widget_selected(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        s.selected_widget = usize::try_from(i).ok();
        load_widgets_into_ui(&ui, &s);
        refresh_preview(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_widget_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        read_widget(&ui, &mut s);
        save(&ui, &s);
        // Only the list labels can change; keep the property controls as they are.
        if let Some(d) = s.cfg.stats.custom_design() {
            let names: Vec<SharedString> = d.widgets.iter().map(|w| widget_label(w, s.prefs.language, &s.readings).into()).collect();
            ui.set_widget_names(ModelRc::new(VecModel::from(names)));
        }
        refresh_preview(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_move_widget(move |dir| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let Some(i) = s.selected_widget else { return };
        let Some(d) = s.cfg.stats.custom_design_mut() else { return };
        let j = i as i32 + dir.signum();
        if j < 0 || j as usize >= d.widgets.len() {
            return;
        }
        d.widgets.swap(i, j as usize);
        s.selected_widget = Some(j as usize);
        edited(&ui, &mut s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_delete_widget(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let Some(i) = s.selected_widget else { return };
        if let Some(d) = s.cfg.stats.custom_design_mut() {
            if i < d.widgets.len() {
                d.widgets.remove(i);
            }
        }
        s.selected_widget = None;
        edited(&ui, &mut s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_preview_pressed(move |x, y| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let (cx, cy) = to_canvas(x, y);
        let values = s.values();
        let hit = s.cfg.stats.custom_design().and_then(|d| overlay::hit_test(d, &values, chrono::Local::now(), cx, cy));
        s.selected_widget = hit;
        s.drag_offset = hit.and_then(|i| s.cfg.stats.custom_design()?.widgets.get(i).map(|w| (w.x - cx, w.y - cy)));
        load_widgets_into_ui(&ui, &s);
        refresh_preview(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_preview_dragged(move |x, y| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let (Some(i), Some((ox, oy))) = (s.selected_widget, s.drag_offset) else { return };
        let (cx, cy) = to_canvas(x, y);
        let max = SCREEN_SIZE as f32;
        let Some(w) = s.cfg.stats.custom_design_mut().and_then(|d| d.widgets.get_mut(i)) else { return };
        w.x = (cx + ox).round().clamp(0.0, max);
        w.y = (cy + oy).round().clamp(0.0, max);
        let (wx, wy) = (w.x, w.y);
        ui.set_w_x(wx);
        ui.set_w_y(wy);
        refresh_preview(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_preview_released(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        if s.drag_offset.take().is_some() {
            save(&ui, &s);
            refresh_gallery(&ui, &s);
        }
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_builtin_background(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let Some((name, png)) = BACKGROUNDS.get(i.max(0) as usize) else { return };
        let mut s = st.borrow_mut();
        let file = format!("media-{}.png", media::now_ms());
        if let Err(e) = std::fs::create_dir_all(config::data_dir()).and_then(|_| std::fs::write(config::data_dir().join(&file), png)) {
            show_message(&ui, false, format!("{}: {e}", if s.prefs.language == 1 { "Arka plan kaydedilemedi" } else { "Could not save background" }));
            return;
        }
        let old = s.cfg.media_file.replace(file);
        s.cfg.media_is_video = false;
        s.cfg.media_source = Some(format!("{name}{BUILTIN_SUFFIX}"));
        s.cfg.media_version += 1;
        save(&ui, &s);
        if let Some(old) = old.filter(|o| Some(o) != s.cfg.media_file.as_ref()) {
            let _ = std::fs::remove_file(config::data_dir().join(old));
        }
        if let Ok(img) = image::load_from_memory(png) {
            s.preview_bg = img.to_rgba8();
        }
        ui.set_media_name(media_label(&s.cfg, s.prefs.language).into());
        ui.set_bg_selected(i);
        show_message(&ui, true, format!("{name} {}", if s.prefs.language == 1 { "soğutucuya gönderildi." } else { "sent to the cooler." }));
        refresh_preview(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_browse(move || {
        let Some(ui) = weak.upgrade() else { return };
        let exts: Vec<&str> = IMAGE_EXTENSIONS.iter().chain(VIDEO_EXTENSIONS).copied().collect();
        let language = st.borrow().prefs.language;
        let Some(p) = rfd::FileDialog::new().add_filter(if language == 1 { "Görseller ve videolar" } else { "Images and videos" }, &exts).pick_file() else { return };
        ui.set_source_path(p.display().to_string().into());
        ui.set_source_is_video(media::is_video(&p));
        let s = st.borrow();
        if media::is_video(&p) && s.ffmpeg.is_none() {
            return; // the Background card offers the ffmpeg download
        }
        let fit = FITS[ui.get_fit().clamp(0, 2) as usize];
        s.spawn(move || Job::Poster(poster_for(&p, fit)));
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_apply_media(move || {
        let Some(ui) = weak.upgrade() else { return };
        let src = PathBuf::from(ui.get_source_path().trim());
        let language = st.borrow().prefs.language;
        if !src.exists() {
            show_message(&ui, false, format!("{}: {}", if language == 1 { "Dosya bulunamadı" } else { "File not found" }, src.display()));
            return;
        }
        let fit = FITS[ui.get_fit().clamp(0, 2) as usize];
        ui.set_busy(true);
        ui.set_busy_text((if media::is_video(&src) { if language == 1 { "Video dönüştürülüyor…" } else { "Converting video…" } } else if language == 1 { "Görsel hazırlanıyor…" } else { "Preparing image…" }).into());
        ui.set_message(SharedString::new());
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        st.borrow().spawn(move || Job::Media(prepare_into_data_dir(&src, fit), name));
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_source_selected(move |i, idx| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let i = i.clamp(0, (Stat::ALL.len() - 1) as i32) as usize;
        let key = usize::try_from(idx - 1).ok().and_then(|k| s.row_keys.get(i)?.get(k).cloned());
        s.cfg.stats.set_source(Stat::ALL[i], key);
        save(&ui, &s);
        s.rows_sig.clear();
        refresh_preview(&ui, &s);
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_repair_service(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_busy(true);
        let language = st.borrow().prefs.language;
        ui.set_busy_text((if language == 1 { "Hizmet yeniden başlatılıyor…" } else { "Restarting service…" }).into());
        show_message(&ui, true, if language == 1 { "Arka plan hizmetini yeniden başlatmak için yönetici iznini onayla." } else { "Approve the admin prompt to restart the background service." });
        st.borrow().spawn(|| Job::ServiceRestarted(elevate("restart", &result_file("restart"))));
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_autostart_toggled(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_autostart_busy(true);
        let what = if on { "set-autostart on" } else { "set-autostart off" };
        st.borrow().spawn(move || Job::Autostart(elevate(what, &result_file("autostart"))));
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_show_tray_toggled(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        if !on {
            // Without a tray icon, a sign-in start would leave the app invisible.
            let _ = desktop::set_open_at_login(false);
        }
        s.prefs.show_tray = on;
        if let Err(e) = s.prefs.save() {
            show_message(&ui, false, format!("{}: {e}", if s.prefs.language == 1 { "Ayar kaydedilemedi" } else { "Could not save preference" }));
        }
        set_tray(&ui, &mut s, on);
    });

    let weak = ui.as_weak();
    ui.on_login_toggled(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        if let Err(e) = desktop::set_open_at_login(on) {
            show_message(&ui, false, e);
        }
        ui.set_login_tray(desktop::open_at_login());
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_download_ffmpeg(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let file = result_file("fetch-ffmpeg");
        s.ffmpeg_progress_file = Some(PathBuf::from(format!("{}.progress", file.display())));
        ui.set_ffmpeg_busy(true);
        ui.set_ffmpeg_progress(0.0);
        ui.set_ffmpeg_state("Downloading… (approve the admin prompt)".into());
        s.spawn(move || Job::FfmpegDownloaded(elevate("fetch-ffmpeg", &file)));
    });

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_lhm_port_edited(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        if let Ok(port) = text.trim().parse::<u16>() {
            let mut s = st.borrow_mut();
            s.cfg.lhm_port = port;
            save(&ui, &s);
        }
    });

    ui.on_open_data_folder(|| {
        let _ = Command::new("explorer.exe").arg(config::data_dir()).spawn();
    });
    ui.on_open_log(|| {
        let _ = Command::new("notepad.exe").arg(config::data_dir().join("service.log")).spawn();
    });
    ui.on_open_url(|url| open_url(&url));

    let weak = ui.as_weak();
    let st = state.clone();
    ui.on_toggle_display(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut s = st.borrow_mut();
        let off = !s.cfg.display_off;
        set_display_off(&ui, &mut s, off);
    });
}

// ---------------------------------------------------------------------------------------
// Periodic updates

fn tick(ui: &AppWindow, state: &Rc<RefCell<State>>) {
    let mut s = state.borrow_mut();

    let actions = s.tray.as_ref().map(Tray::poll).unwrap_or_default();
    for action in actions {
        match action {
            TrayAction::Open => show_window(ui.clone_strong()),
            TrayAction::ToggleStats(on) => {
                s.cfg.stats.enabled = on;
                save(ui, &s);
                ui.set_stats_enabled(on);
                refresh_preview(ui, &s);
            }
            TrayAction::ToggleDisplay => {
                let off = !s.cfg.display_off;
                set_display_off(ui, &mut s, off);
            }
            TrayAction::Quit => {
                let _ = slint::quit_event_loop();
                return;
            }
        }
    }
    if s.last_theme_check.elapsed() >= Duration::from_secs(5) {
        s.last_theme_check = Instant::now();
        if let Some(t) = s.tray.as_mut() {
            t.refresh_theme();
        }
    }

    while let Ok(job) = s.jobs.1.try_recv() {
        match job {
            Job::Poster(Ok(img)) => {
                s.preview_bg = img;
                refresh_preview(ui, &s);
            }
            Job::Poster(Err(e)) => show_message(ui, false, e),
            Job::Media(result, source_name) => {
                ui.set_busy(false);
                match result {
                    Ok((file, is_video, poster)) => {
                        let old = s.cfg.media_file.replace(file);
                        s.cfg.media_is_video = is_video;
                        s.cfg.media_source = Some(source_name);
                        s.cfg.media_version += 1;
                        match s.cfg.save() {
                            Ok(()) => {
                                if let Some(old) = old.filter(|o| Some(o) != s.cfg.media_file.as_ref()) {
                                    let _ = std::fs::remove_file(config::data_dir().join(old));
                                }
                                show_message(ui, true, if s.prefs.language == 1 { "Soğutucuya gönderildi. Birkaç saniye içinde güncellenir." } else { "Sent to the cooler. It will update in a few seconds." });
                            }
                            Err(e) => show_message(ui, false, format!("{}: {e}", if s.prefs.language == 1 { "Ayarlar kaydedilemedi" } else { "Could not save settings" })),
                        }
                        ui.set_media_name(media_label(&s.cfg, s.prefs.language).into());
                        ui.set_bg_selected(builtin_index(&s.cfg));
                        s.preview_bg = poster;
                        refresh_preview(ui, &s);
                    }
                    Err(e) => show_message(ui, false, e),
                }
            }
            Job::Autostart(r) => {
                ui.set_autostart_busy(false);
                if let Err(e) = r {
                    show_message(ui, false, e);
                }
                ui.set_autostart(service::query_autostart().unwrap_or(false));
            }
            Job::ServiceRestarted(r) => {
                ui.set_busy(false);
                match r {
                    Ok(()) => show_message(ui, true, if s.prefs.language == 1 { "Arka plan hizmeti yeniden başlatıldı." } else { "Background service restarted." }),
                    Err(e) => show_message(ui, false, e),
                }
                s.last_poll = Instant::now() - Duration::from_secs(10);
            }
            Job::FfmpegFound(path) => {
                s.ffmpeg = path;
                update_ffmpeg_ui(ui, &s);
            }
            Job::FfmpegDownloaded(r) => {
                ui.set_ffmpeg_busy(false);
                s.ffmpeg_progress_file = None;
                if let Err(e) = r {
                    show_message(ui, false, format!("{}: {e}", if s.prefs.language == 1 { "ffmpeg indirme başarısız" } else { "ffmpeg download failed" }));
                }
                s.spawn(|| Job::FfmpegFound(ffmpeg::find()));
            }
        }
    }

    if let Some(p) = &s.ffmpeg_progress_file {
        if let Ok(text) = std::fs::read_to_string(p) {
            let mut it = text.split_whitespace().filter_map(|x| x.parse::<f64>().ok());
            if let (Some(done), Some(total)) = (it.next(), it.next()) {
                if total > 0.0 {
                    ui.set_ffmpeg_progress((done / total) as f32);
                    ui.set_ffmpeg_state(format!("Downloading… {:.0} / {:.0} MB", done / 1e6, total / 1e6).into());
                }
            }
        }
    }

    // Re-render the preview background when the fit changes for a chosen file.
    if ui.get_fit() != s.last_fit {
        s.last_fit = ui.get_fit();
        let src = PathBuf::from(ui.get_source_path().trim());
        if src.exists() && (!media::is_video(&src) || s.ffmpeg.is_some()) {
            let fit = FITS[s.last_fit.clamp(0, 2) as usize];
            s.spawn(move || Job::Poster(poster_for(&src, fit)));
        }
    }

    if s.last_poll.elapsed() < Duration::from_secs(1) {
        return;
    }
    s.last_poll = Instant::now();

    let status = Status::load().filter(|st| media::now_ms().saturating_sub(st.updated_ms) < STATUS_STALE_MS);
    let state_now = service::query_state();
    s.live = status.as_ref().is_some_and(|st| !st.readings.is_empty());
    s.readings = status.as_ref().map(|st| st.readings.clone()).unwrap_or_default();

    let lang = s.prefs.language;
    let (svc_text, running) = match state_now {
        None => (if lang == 1 { "Kurulu değil" } else { "Not installed" }, false),
        Some(ServiceState::Running) => (if lang == 1 { "Çalışıyor" } else { "Running" }, true),
        Some(ServiceState::Stopped) => (if lang == 1 { "Durduruldu" } else { "Stopped" }, false),
        Some(ServiceState::StartPending) => (if lang == 1 { "Başlatılıyor…" } else { "Starting…" }, false),
        Some(ServiceState::StopPending) => (if lang == 1 { "Durduruluyor…" } else { "Stopping…" }, false),
        Some(_) => (if lang == 1 { "Meşgul" } else { "Busy" }, false),
    };
    ui.set_service_state(svc_text.into());
    ui.set_service_running(running);
    match &status {
        Some(st) => {
            ui.set_cooler_ok(st.device_connected);
            ui.set_cooler_state(if st.device_connected { if lang == 1 { "Soğutucu bağlı" } else { "Cooler connected" } } else if lang == 1 { "Soğutucu bulunamadı" } else { "Cooler not found" }.into());
            ui.set_sensors_state(st.sensors.clone().into());
            ui.set_sensors_ok(!st.readings.is_empty());
            ui.set_last_error(st.last_error.clone().unwrap_or_default().into());
        }
        None => {
            ui.set_cooler_ok(false);
            ui.set_cooler_state(if running { if lang == 1 { "Hizmet bekleniyor…" } else { "Waiting for service…" } } else if lang == 1 { "Hizmet çalışmıyor" } else { "Service not running" }.into());
            ui.set_sensors_state((if lang == 1 { "Hizmetten henüz veri alınamadı." } else { "No readings from service yet." }).into());
            ui.set_sensors_ok(true);
            ui.set_last_error(SharedString::new());
        }
    }

    if s.last_process_check.elapsed() >= Duration::from_secs(3) {
        s.last_process_check = Instant::now();
        ui.set_official_running(official_app_running());
        if !ui.get_autostart_busy() {
            ui.set_autostart(service::query_autostart().unwrap_or(false));
        }
    }

    if let Some(t) = &s.tray {
        let v = s.cfg.stats.resolve(&s.readings);
        let tip = match (v.get(Stat::CpuTemp), v.get(Stat::GpuTemp)) {
            (Some(c), Some(g)) if s.live => format!("AIO Cooler Screen\nCPU {c:.0}° · GPU {g:.0}°"),
            _ => "AIO Cooler Screen".into(),
        };
        t.set_tooltip(&tip);
    }

    // Nothing to redraw while the window is hidden in the tray.
    if ui.window().is_visible() {
        update_stat_rows(&mut s);
        refresh_preview(ui, &s);
    }
}

fn fmt_value(stat: Stat, v: f64) -> String {
    if stat.is_temp() {
        format!("{v:.0}°")
    } else {
        format!("{v:.0}%")
    }
}

/// Rebuild the source pickers only when the available readings change (so an open
/// drop-down isn't reset every second); otherwise just refresh the values.
fn update_stat_rows(s: &mut State) {
    let mut keys = Vec::new();
    let mut labels = Vec::new();
    let mut sig = format!("lang={};", s.prefs.language);
    for stat in Stat::ALL {
        let candidates: Vec<&Reading> = sorted_sensor_readings(&s.readings)
            .into_iter().filter(|r| r.kind == stat.reading_kind()).collect();
        let k: Vec<String> = candidates.iter().map(|r| r.key.clone()).collect();
        let names: Vec<String> = candidates.iter().map(|r| friendly_sensor_label(r)).collect();
        // Include labels too, so updated hardware names refresh without changing the stored IDs.
        for (key, name) in k.iter().zip(names.iter()) {
            sig.push_str(&format!("{}:{}:{}:{};", key.len(), key, name.len(), name));
        }
        sig.push_str(s.cfg.stats.source(stat).map_or("", |x| x));
        sig.push(';');
        keys.push(k);
        labels.push(names);
    }
    let values = s.values();
    let rebuild = sig != s.rows_sig || s.rows.row_count() != Stat::ALL.len();
    for (i, stat) in Stat::ALL.into_iter().enumerate() {
        let value: SharedString = values.get(stat).map(|v| fmt_value(stat, v)).unwrap_or_else(|| "--".into()).into();
        if !rebuild {
            if let Some(mut row) = s.rows.row_data(i) {
                row.value = value;
                s.rows.set_row_data(i, row);
            }
            continue;
        }
        let auto_prefix = if s.prefs.language == 1 { "Otomatik" } else { "Automatic" };
        let auto = stat.auto_pick(&s.readings).map_or_else(
            || auto_prefix.to_string(),
            |r| format!("{auto_prefix} ({})", friendly_sensor_label(r)),
        );
        let mut choices: Vec<SharedString> = vec![auto.into()];
        choices.extend(labels[i].iter().map(|name| SharedString::from(name.as_str())));
        let index = s.cfg.stats.source(stat).and_then(|src| keys[i].iter().position(|k| k == src)).map_or(0, |p| p as i32 + 1);
        let row = StatRow { title: stat.title().into(), choices: ModelRc::new(VecModel::from(choices)), index, value };
        if i < s.rows.row_count() {
            s.rows.set_row_data(i, row);
        } else {
            s.rows.push(row);
        }
    }
    if rebuild {
        s.row_keys = keys;
        s.rows_sig = sig;
    }
}

// ---------------------------------------------------------------------------------------
// Helpers

/// Convert the user's file and store it in the data dir. Returns (file name, is_video, poster).
fn prepare_into_data_dir(src: &Path, fit: Fit) -> Result<(String, bool, RgbaImage), String> {
    let dir = config::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stamp = media::now_ms();
    if media::is_video(src) {
        let name = format!("media-{stamp}.mp4");
        media::prepare_video(src, fit, &dir.join(&name))?;
        let poster = media::video_poster(&dir.join(&name), Fit::Cover)?;
        Ok((name, true, poster))
    } else {
        let img = media::prepare_image(src, fit)?;
        let name = format!("media-{stamp}.png");
        std::fs::write(dir.join(&name), media::encode_png(&img)?).map_err(|e| format!("cannot save media: {e}"))?;
        Ok((name, false, img))
    }
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(Some(0)).collect()
}

fn open_url(url: &str) {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, file) = (wide("open".as_ref()), wide(url.as_ref()));
    // SAFETY: NUL-terminated wide strings that outlive the call.
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

fn official_app_running() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    // SAFETY: standard process snapshot walk; the entry struct is sized before use.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case("Monster AIO Cooler.exe") {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        found
    }
}

fn result_file(what: &str) -> PathBuf {
    std::env::temp_dir().join(format!("aio-screen-{what}-{}.txt", media::now_ms()))
}

/// Run `aio-screen.exe <what> <result-file>` elevated and wait for it.
fn elevate(what: &str, result_file: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let params = wide(format!("{what} \"{}\"", result_file.display()).as_ref());

    // SAFETY: plain Win32 calls with NUL-terminated wide strings that outlive the call.
    let result = unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            return Err("Yönetici izni iptal edildi.".into());
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        let detail = std::fs::read_to_string(result_file).unwrap_or_default();
        if code == 0 {
            Ok(())
        } else {
            Err(if detail.trim().is_empty() { format!("{what} failed (exit code {code})") } else { detail.trim().to_string() })
        }
    };
    let _ = std::fs::remove_file(result_file);
    let _ = std::fs::remove_file(format!("{}.progress", result_file.display()));
    result
}
