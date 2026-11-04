//! Shared state between the GUI (writes config, prepares media) and the service
//! (reads config, writes status). Lives in %ProgramData%\aio-screen.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::sensors::{Reading, Stat};

pub const SERVICE_NAME: &str = "AioScreen";
pub const SERVICE_DISPLAY_NAME: &str = "AIO Cooler Screen";

pub fn data_dir() -> PathBuf {
    // For testing the GUI against a throwaway folder instead of the real settings.
    if let Some(dir) = std::env::var_os("AIO_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    base.join("aio-screen")
}

/// Where `install` copies the executable the service runs from (admin-only location).
pub fn install_dir() -> PathBuf {
    let base = std::env::var_os("ProgramFiles").map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
    base.join("AIO Screen")
}

fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

fn status_path() -> PathBuf {
    data_dir().join("status.json")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Position {
    Top,
    Center,
    Bottom,
}

/// The screen designs drawn over the background: four built-in ones and the user's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Design {
    Gauges,
    Tiles,
    BigStat,
    Clock,
    /// A user-made design, by `CustomDesign::id`.
    Custom(u64),
}

impl Design {
    /// The built-in designs, in gallery order.
    pub const ALL: [Design; 4] = [Design::Gauges, Design::Tiles, Design::BigStat, Design::Clock];

    pub fn name(self) -> &'static str {
        match self {
            Design::Gauges => "Ring gauges",
            Design::Tiles => "Dashboard tiles",
            Design::BigStat => "Big single stat",
            Design::Clock => "Clock and stats",
            Design::Custom(_) => "Custom",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WidgetKind {
    Value,
    Label,
    Ring,
    Bar,
    Time,
    Date,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 6] =
        [WidgetKind::Value, WidgetKind::Label, WidgetKind::Ring, WidgetKind::Bar, WidgetKind::Time, WidgetKind::Date];

    pub fn name(self) -> &'static str {
        match self {
            WidgetKind::Value => "Stat value",
            WidgetKind::Label => "Text",
            WidgetKind::Ring => "Ring gauge",
            WidgetKind::Bar => "Progress bar",
            WidgetKind::Time => "Time",
            WidgetKind::Date => "Date",
        }
    }

    pub fn uses_stat(self) -> bool {
        matches!(self, WidgetKind::Value | WidgetKind::Ring | WidgetKind::Bar)
    }

    pub fn is_text(self) -> bool {
        matches!(self, WidgetKind::Value | WidgetKind::Label | WidgetKind::Time | WidgetKind::Date)
    }
}

/// One element of a custom design. Coordinates are the widget's centre on the 480x480 screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Widget {
    pub kind: WidgetKind,
    pub x: f32,
    pub y: f32,
    /// Text size, ring radius or bar width, in pixels.
    pub size: f32,
    /// Ring or bar thickness, in pixels.
    pub thickness: f32,
    pub stat: Stat,
    /// Optional direct sensor source for custom widgets (legacy stat is the fallback).
    pub sensor_key: Option<String>,
    /// Maximum used to scale a ring/bar for non-percent sensors.
    pub sensor_max: f64,
    /// Text for label widgets.
    pub text: String,
    pub color: [u8; 3],
    /// Use the design's colours (text colour, or temperature/accent colour for gauges)
    /// instead of `color`.
    pub auto_color: bool,
    pub bold: bool,
}

impl Default for Widget {
    fn default() -> Self {
        Widget {
            kind: WidgetKind::Value,
            x: 240.0,
            y: 240.0,
            size: 90.0,
            thickness: 14.0,
            stat: Stat::CpuTemp,
            sensor_key: None,
            sensor_max: 100.0,
            text: "CPU".into(),
            color: [255, 255, 255],
            auto_color: true,
            bold: true,
        }
    }
}

impl Widget {
    /// A sensible new widget of `kind`, placed near the centre.
    pub fn new(kind: WidgetKind) -> Self {
        let base = Widget { kind, ..Widget::default() };
        match kind {
            WidgetKind::Value => base,
            WidgetKind::Label => Widget { size: 34.0, bold: false, text: "CPU".into(), ..base },
            WidgetKind::Ring => Widget { size: 200.0, thickness: 18.0, ..base },
            WidgetKind::Bar => Widget { y: 330.0, size: 220.0, thickness: 12.0, ..base },
            WidgetKind::Time => Widget { size: 110.0, ..base },
            WidgetKind::Date => Widget { y: 300.0, size: 32.0, bold: false, ..base },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomDesign {
    pub id: u64,
    pub name: String,
    pub look: Look,
    /// 24-hour time for time widgets (12-hour shows AM/PM).
    pub h24: bool,
    pub widgets: Vec<Widget>,
}

impl Default for CustomDesign {
    fn default() -> Self {
        CustomDesign { id: 0, name: "My design".into(), look: Look::default(), h24: true, widgets: Vec::new() }
    }
}

impl CustomDesign {
    /// A new design with a starter layout: CPU temperature ring, label, value and load.
    pub fn starter(id: u64, name: String) -> Self {
        let w = Widget::new;
        CustomDesign {
            id,
            name,
            widgets: vec![
                w(WidgetKind::Ring),
                Widget { y: 170.0, ..w(WidgetKind::Label) },
                Widget { y: 245.0, size: 110.0, ..w(WidgetKind::Value) },
                Widget { y: 320.0, size: 34.0, bold: false, stat: Stat::CpuLoad, ..w(WidgetKind::Value) },
            ],
            ..CustomDesign::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsConfig {
    pub enabled: bool,
    pub design: Design,
    pub gauges: GaugesOptions,
    pub tiles: TilesOptions,
    pub big_stat: BigStatOptions,
    pub clock: ClockOptions,
    pub custom: Vec<CustomDesign>,
    /// Per-stat reading key chosen by the user; missing means automatic.
    pub sources: BTreeMap<String, String>,
}

impl Default for StatsConfig {
    fn default() -> Self {
        StatsConfig {
            enabled: true,
            design: Design::Gauges,
            gauges: GaugesOptions::default(),
            tiles: TilesOptions::default(),
            big_stat: BigStatOptions::default(),
            clock: ClockOptions::default(),
            custom: Vec::new(),
            sources: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FontChoice {
    SegoeUi,
    Bahnschrift,
    Arial,
    Consolas,
    GoogleSans,
    GoogleSansFlex,
    Inter,
    Rajdhani,
    Orbitron,
    JetBrainsMono,
    RobotoCondensed,
    Oswald,
}

impl FontChoice {
    pub const ALL: [FontChoice; 12] = [FontChoice::SegoeUi, FontChoice::Bahnschrift, FontChoice::Arial, FontChoice::Consolas, FontChoice::GoogleSans, FontChoice::GoogleSansFlex, FontChoice::Inter, FontChoice::Rajdhani, FontChoice::Orbitron, FontChoice::JetBrainsMono, FontChoice::RobotoCondensed, FontChoice::Oswald];
}

/// Colours and text styling; every design keeps its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub text: [u8; 3],
    /// Colour of gauges, rings and highlights.
    pub accent: [u8; 3],
    /// Colour temperatures from blue (cool) to red (hot) instead of the accent.
    pub temp_colors: bool,
    /// Darken the background by this many percent so stats stay readable.
    pub dim: u8,
    pub font: FontChoice,
    /// Text size in percent of the design's default.
    pub text_scale: u8,
    /// Dark outline around text.
    pub outline: bool,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            text: [255, 255, 255],
            accent: [56, 189, 248],
            temp_colors: true,
            dim: 35,
            font: FontChoice::SegoeUi,
            text_scale: 100,
            outline: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GaugesOptions {
    pub look: Look,
    pub load_rings: bool,
    pub labels: bool,
    /// Temperature ring thickness in pixels.
    pub thickness: u8,
}

impl Default for GaugesOptions {
    fn default() -> Self {
        GaugesOptions { look: Look::default(), load_rings: true, labels: true, thickness: 20 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TilesOptions {
    pub look: Look,
    pub show: Vec<Stat>,
    pub position: Position,
    pub columns: u8,
    pub panel: bool,
    /// Panel opacity in percent.
    pub panel_opacity: u8,
}

impl Default for TilesOptions {
    fn default() -> Self {
        TilesOptions {
            look: Look { dim: 0, ..Look::default() },
            show: Stat::ALL.to_vec(),
            position: Position::Center,
            columns: 2,
            panel: true,
            panel_opacity: 65,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BigStatOptions {
    pub look: Look,
    pub stat: Stat,
    pub ring: bool,
    pub ticks: bool,
    /// Show the related stat (e.g. load under temperature) underneath.
    pub secondary: bool,
}

impl Default for BigStatOptions {
    fn default() -> Self {
        BigStatOptions { look: Look::default(), stat: Stat::CpuTemp, ring: true, ticks: true, secondary: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClockStats {
    Temps,
    Loads,
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockOptions {
    pub look: Look,
    pub h24: bool,
    pub seconds_ring: bool,
    pub date: bool,
    pub stats: ClockStats,
}

impl Default for ClockOptions {
    fn default() -> Self {
        ClockOptions { look: Look::default(), h24: true, seconds_ring: true, date: true, stats: ClockStats::Temps }
    }
}

/// Current value of every stat (None when no reading is available).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatValues(pub [Option<f64>; 4], pub Vec<Reading>);

impl StatValues {
    pub fn widget_reading(&self, widget: &Widget) -> Option<&Reading> {
        widget.sensor_key.as_ref().and_then(|key| self.1.iter().find(|r| &r.key == key))
    }

    pub fn widget_value(&self, widget: &Widget) -> Option<f64> {
        if widget.sensor_key.is_some() {
            self.widget_reading(widget).map(|r| r.value)
        } else {
            self.get(widget.stat)
        }
    }

    pub fn widget_fraction(&self, widget: &Widget) -> f32 {
        let value = self.widget_value(widget).unwrap_or(0.0);
        let max = if widget.sensor_key.is_some() { widget.sensor_max } else { 100.0 };
        if max.is_finite() && max > 0.0 { (value / max).clamp(0.0, 1.0) as f32 } else { 0.0 }
    }

    pub fn widget_text(&self, widget: &Widget) -> String {
        if widget.sensor_key.is_none() {
            return match self.get(widget.stat) {
                Some(v) if widget.stat.is_temp() => format!("{v:.0}°"),
                Some(v) => format!("{v:.0}%"),
                None => "--".into(),
            };
        }
        match self.widget_reading(widget) {
            Some(r) => {
                let formatted = if r.value.abs() < 10.0 && r.kind != 3 && r.kind != 6 {
                    format!("{:.2}", r.value)
                } else { format!("{:.0}", r.value) };
                if r.units.is_empty() { formatted } else { format!("{} {}", formatted, r.units) }
            }
            None => "--".into(),
        }
    }

    pub fn get(&self, stat: Stat) -> Option<f64> {
        self.0[Stat::ALL.iter().position(|s| *s == stat).unwrap_or(0)]
    }

    /// Plausible values for previews before the service reports real ones.
    pub fn sample() -> Self {
        StatValues([Some(58.0), Some(23.0), Some(47.0), Some(12.0)], Vec::new())
    }
}

impl StatsConfig {
    /// The selected custom design, if a custom design is selected and still exists.
    pub fn custom_design(&self) -> Option<&CustomDesign> {
        match self.design {
            Design::Custom(id) => self.custom.iter().find(|d| d.id == id),
            _ => None,
        }
    }

    pub fn custom_design_mut(&mut self) -> Option<&mut CustomDesign> {
        match self.design {
            Design::Custom(id) => self.custom.iter_mut().find(|d| d.id == id),
            _ => None,
        }
    }

    /// The look of the selected design.
    pub fn look(&self) -> &Look {
        match self.design {
            Design::Gauges => &self.gauges.look,
            Design::Tiles => &self.tiles.look,
            Design::BigStat => &self.big_stat.look,
            Design::Clock => &self.clock.look,
            Design::Custom(_) => self.custom_design().map_or(&self.gauges.look, |d| &d.look),
        }
    }

    pub fn look_mut(&mut self) -> &mut Look {
        match self.design {
            Design::Gauges => &mut self.gauges.look,
            Design::Tiles => &mut self.tiles.look,
            Design::BigStat => &mut self.big_stat.look,
            Design::Clock => &mut self.clock.look,
            Design::Custom(id) => match self.custom.iter().position(|d| d.id == id) {
                Some(i) => &mut self.custom[i].look,
                None => &mut self.gauges.look,
            },
        }
    }

    pub fn source(&self, stat: Stat) -> Option<&String> {
        self.sources.get(&format!("{stat:?}"))
    }

    pub fn set_source(&mut self, stat: Stat, key: Option<String>) {
        let k = format!("{stat:?}");
        match key {
            Some(v) => self.sources.insert(k, v),
            None => self.sources.remove(&k),
        };
    }

    /// The reading that feeds `stat`: the user's choice, or the automatic pick.
    pub fn reading<'a>(&self, stat: Stat, readings: &'a [Reading]) -> Option<&'a Reading> {
        match self.source(stat) {
            Some(key) => readings.iter().find(|r| &r.key == key),
            None => stat.auto_pick(readings),
        }
    }

    pub fn resolve(&self, readings: &[Reading]) -> StatValues {
        StatValues(Stat::ALL.map(|s| self.reading(s, readings).map(|r| r.value)), readings.to_vec())
    }
}

pub const DEFAULT_LHM_PORT: u16 = 8085;

fn default_lhm_port() -> u16 {
    DEFAULT_LHM_PORT
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// LibreHardwareMonitor web server port.
    #[serde(default = "default_lhm_port")]
    pub lhm_port: u16,
    /// File name inside the data dir of the prepared media (480x480 PNG or MP4).
    pub media_file: Option<String>,
    /// Original file name the user picked, for display.
    pub media_source: Option<String>,
    pub media_is_video: bool,
    /// Bumped by the GUI on every change so the service re-uploads.
    pub media_version: u64,
    pub stats: StatsConfig,
    /// The user turned the screen off: the service blanks it and stops waking it.
    pub display_off: bool,
}

impl Config {
    pub fn effective_lhm_port(&self) -> u16 {
        if self.lhm_port == 0 {
            DEFAULT_LHM_PORT
        } else {
            self.lhm_port
        }
    }

    pub fn load() -> Config {
        let Ok(text) = std::fs::read_to_string(config_path()) else { return Config::default() };
        serde_json::from_str(&text)
            .ok()
            .or_else(|| serde_json::from_str(&text).ok().map(migrate_v1))
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        write_atomic(config_path(), &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
    }

    pub fn modified() -> Option<std::time::SystemTime> {
        std::fs::metadata(config_path()).and_then(|m| m.modified()).ok()
    }
}

/// Convert a 1.0 config (one shared look, flat design options) to the current layout,
/// keeping the background and as many settings as map across.
fn migrate_v1(v: serde_json::Value) -> Config {
    use serde_json::from_value as from;
    let get = |o: &serde_json::Value, k: &str| o.get(k).cloned().unwrap_or(serde_json::Value::Null);
    let mut cfg = Config {
        lhm_port: from(get(&v, "lhm_port")).unwrap_or(DEFAULT_LHM_PORT),
        media_file: from(get(&v, "media_file")).unwrap_or_default(),
        media_source: from(get(&v, "media_source")).unwrap_or_default(),
        media_is_video: from(get(&v, "media_is_video")).unwrap_or_default(),
        media_version: from(get(&v, "media_version")).unwrap_or_default(),
        stats: StatsConfig::default(),
        display_off: false,
    };
    let s = get(&v, "stats");
    let st = &mut cfg.stats;
    st.enabled = from(get(&s, "enabled")).unwrap_or(true);
    st.design = from(get(&s, "design")).unwrap_or(Design::Gauges);
    st.sources = from(get(&s, "sources")).unwrap_or_default();
    for look in [&mut st.gauges.look, &mut st.tiles.look, &mut st.big_stat.look, &mut st.clock.look] {
        look.text = from(get(&s, "color")).unwrap_or(look.text);
        look.accent = from(get(&s, "accent")).unwrap_or(look.accent);
        look.temp_colors = from(get(&s, "temp_colors")).unwrap_or(look.temp_colors);
    }
    for look in [&mut st.gauges.look, &mut st.big_stat.look, &mut st.clock.look] {
        look.dim = from(get(&s, "dim")).unwrap_or(look.dim);
    }
    st.tiles.show = from(get(&s, "show")).unwrap_or(st.tiles.show.clone());
    st.tiles.position = from(get(&s, "position")).unwrap_or(st.tiles.position);
    st.tiles.panel = from(get(&s, "panel")).unwrap_or(st.tiles.panel);
    st.big_stat.stat = from(get(&s, "big_stat")).unwrap_or(st.big_stat.stat);
    st.clock.h24 = from(get(&s, "clock_24h")).unwrap_or(st.clock.h24);
    cfg
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub updated_ms: u128,
    pub device_connected: bool,
    pub sensors: String,
    pub media_version_applied: Option<u64>,
    pub last_error: Option<String>,
    /// Temperature and usage readings, for the GUI's source pickers and preview.
    pub readings: Vec<Reading>,
}

impl Status {
    pub fn load() -> Option<Status> {
        std::fs::read_to_string(status_path()).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn save(&self) -> Result<(), String> {
        write_atomic(status_path(), &serde_json::to_vec(self).map_err(|e| e.to_string())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_v1_config_keeping_background_and_settings() {
        let v1 = r#"{"media_file":"media-1.png","media_source":null,"media_is_video":false,"media_version":9,
            "stats":{"enabled":true,"design":"Tiles","show":["CpuTemp","GpuTemp"],"position":"Bottom",
            "color":[255,0,0],"accent":[56,189,248],"temp_colors":false,"panel":true,"dim":35,
            "big_stat":"GpuLoad","clock_24h":false,"sources":{}}}"#;
        assert!(serde_json::from_str::<Config>(v1).is_err(), "v1 should need migrating");
        let cfg = migrate_v1(serde_json::from_str(v1).unwrap());
        assert_eq!(cfg.media_file.as_deref(), Some("media-1.png"));
        assert_eq!(cfg.media_version, 9);
        assert_eq!(cfg.stats.design, Design::Tiles);
        assert_eq!(cfg.stats.tiles.show, vec![Stat::CpuTemp, Stat::GpuTemp]);
        assert_eq!(cfg.stats.tiles.position, Position::Bottom);
        assert_eq!(cfg.stats.tiles.look.text, [255, 0, 0]);
        assert!(!cfg.stats.gauges.look.temp_colors);
        assert_eq!(cfg.stats.big_stat.stat, Stat::GpuLoad);
        assert!(!cfg.stats.clock.h24);
        assert_eq!(cfg.effective_lhm_port(), DEFAULT_LHM_PORT);
    }
}

fn write_atomic(path: PathBuf, bytes: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(data_dir()).map_err(|e| format!("cannot create {}: {e}", data_dir().display()))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}
