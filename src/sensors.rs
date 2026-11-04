//! Sensor readings, auto-detected:
//! - built in: CPU temperature through PawnIO and CPU load from Windows (see cpu.rs),
//! - NVIDIA GPUs directly through NVML (nvml.dll, installed with the NVIDIA driver),
//! - optionally HWiNFO with "Shared Memory Support" enabled (Global\HWiNFO_SENS_SM2),
//! - optionally LibreHardwareMonitor's web server (http://127.0.0.1:<port>/data.json).

use std::ffi::c_char;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use libloading::os::windows::Library;
use serde::{Deserialize, Serialize};

use crate::cpu::{CpuLoad, CpuTemp};

// HWiNFO SENSOR_READING_TYPE values (also used for our own Reading::kind).
pub const READING_TEMP: i32 = 1;
pub const READING_USAGE: i32 = 7;
const READING_TYPES: [(i32, &str); 8] = [
    (1, "temp"),
    (2, "volt"),
    (3, "fan"),
    (4, "current"),
    (5, "power"),
    (6, "clock"),
    (7, "usage"),
    (8, "other"),
];
pub const LOC_OTHER: i32 = 0;
pub const LOC_CPU: i32 = 11;
pub const LOC_GPU: i32 = 13;

const NVML_RETRY: Duration = Duration::from_secs(60);

/// One sensor value, flattened for display and selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    /// Stable identifier: "<source> · <sensor> / <reading>".
    pub key: String,
    pub sensor: String,
    pub name: String,
    pub kind: i32,
    pub sensor_location: i32,
    pub value: f64,
    pub units: String,
}

impl Reading {
    fn new(source: &str, sensor: &str, name: &str, kind: i32, location: i32, value: f64, units: &str) -> Self {
        let sensor = format!("{source} · {sensor}");
        Reading {
            key: format!("{sensor} / {name}"),
            sensor,
            name: name.to_string(),
            kind,
            sensor_location: location,
            value,
            units: units.to_string(),
        }
    }
}

fn location_from_name(name: &str) -> i32 {
    let n = name.trim_start().to_ascii_uppercase();
    if n.starts_with("CPU") {
        LOC_CPU
    } else if n.starts_with("GPU") {
        LOC_GPU
    } else {
        LOC_OTHER
    }
}

fn c_str(buf: &[u8]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

// ---------------------------------------------------------------------------------------
// HWiNFO shared memory

const HWINFO_SIGNATURE: u32 = 0x5369_5748; // "HWiS"

/// Read every temperature/usage reading from HWiNFO's shared memory, if it is running
/// with Shared Memory Support enabled.
pub fn read_hwinfo(kinds: &[i32]) -> Result<Vec<Reading>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Memory::{
        MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ, MEMORY_BASIC_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::{OpenMutexW, ReleaseMutex, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE};

    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    // SAFETY: read-only mapping of HWiNFO's documented shared memory; every offset is
    // bounds-checked against the mapped region size before it is read.
    unsafe {
        let map = OpenFileMappingW(FILE_MAP_READ, 0, wide(r"Global\HWiNFO_SENS_SM2").as_ptr());
        if map.is_null() {
            return Err("not running (enable Shared Memory Support in HWiNFO)".into());
        }
        let view = MapViewOfFile(map, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(map);
            return Err("cannot map shared memory".into());
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        VirtualQuery(view.Value, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        // Copy under HWiNFO's mutex so the snapshot is consistent.
        let mutex = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, wide(r"Global\HWiNFO_SM2_MUTEX").as_ptr());
        let locked = !mutex.is_null() && WaitForSingleObject(mutex, 100) == WAIT_OBJECT_0;
        let data = std::slice::from_raw_parts(view.Value as *const u8, mbi.RegionSize).to_vec();
        if locked {
            ReleaseMutex(mutex);
        }
        if !mutex.is_null() {
            CloseHandle(mutex);
        }
        UnmapViewOfFile(view);
        CloseHandle(map);
        parse_hwinfo(&data, kinds)
    }
}

fn parse_hwinfo(data: &[u8], kinds: &[i32]) -> Result<Vec<Reading>, String> {
    let u32_at = |off: usize| data.get(off..off + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let f64_at = |off: usize| data.get(off..off + 8).map(|b| f64::from_le_bytes(b.try_into().unwrap()));
    if u32_at(0) != Some(HWINFO_SIGNATURE) {
        return Err("shared memory is not active".into());
    }
    let field = |off| u32_at(off).map(|v| v as usize).ok_or("truncated header");
    let (s_off, s_size, s_num) = (field(20)?, field(24)?, field(28)?);
    let (r_off, r_size, r_num) = (field(32)?, field(36)?, field(40)?);
    if s_size < 264 || r_size < 292 {
        return Err("unsupported shared memory layout".into());
    }
    let sensor_names: Vec<String> = (0..s_num)
        .map(|i| {
            let base = s_off + i * s_size;
            let user = data.get(base + 136..base + 264).map(c_str).unwrap_or_default();
            if user.is_empty() {
                data.get(base + 8..base + 136).map(c_str).unwrap_or_default()
            } else {
                user
            }
        })
        .collect();
    let mut out = Vec::new();
    for i in 0..r_num {
        let base = r_off + i * r_size;
        let (Some(kind), Some(sensor_idx), Some(value)) = (u32_at(base), u32_at(base + 4), f64_at(base + 284)) else { break };
        let kind = kind as i32;
        if !kinds.is_empty() && !kinds.contains(&kind) {
            continue;
        }
        let label = data.get(base + 140..base + 268).map(c_str).filter(|s| !s.is_empty());
        let label = label.or_else(|| data.get(base + 12..base + 140).map(c_str)).unwrap_or_default();
        let units = data.get(base + 268..base + 284).map(c_str).unwrap_or_default();
        let sensor = sensor_names.get(sensor_idx as usize).cloned().unwrap_or_default();
        out.push(Reading::new("HWiNFO", &sensor, &label, kind, location_from_name(&sensor), value, &units));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// LibreHardwareMonitor web server

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LhmNode {
    #[serde(default)]
    text: String,
    #[serde(default)]
    children: Vec<LhmNode>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default, rename = "Type")]
    kind: Option<String>,
    #[serde(default)]
    sensor_id: Option<String>,
}

/// "58,3 °C" -> (58.3, "°C")
fn parse_lhm_value(s: &str) -> Option<(f64, String)> {
    let s = s.trim();
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.' || c == '-')).unwrap_or(s.len());
    let value = s[..end].replace(',', ".").parse().ok()?;
    Some((value, s[end..].trim().to_string()))
}

fn walk_lhm(node: &LhmNode, depth: usize, hardware: &str, kinds: &[i32], out: &mut Vec<Reading>) {
    // Depth 0 is the root, 1 the computer, 2 each piece of hardware.
    let hardware = if depth == 2 { node.text.as_str() } else { hardware };
    if let (Some(kind), Some(value), Some(id)) = (&node.kind, &node.value, &node.sensor_id) {
        let kind = match kind.as_str() {
            "Temperature" => READING_TEMP,
            "Load" => READING_USAGE,
            "Voltage" => 2,
            "Fan" => 3,
            "Current" => 4,
            "Power" => 5,
            "Clock" => 6,
            "Data" | "SmallData" | "Throughput" | "Control" | "Level" | "Factor" => 8,
            _ => 0,
        };
        if kind != 0 && (kinds.is_empty() || kinds.contains(&kind)) {
            if let Some((v, units)) = parse_lhm_value(value) {
                let location = if id.contains("cpu") {
                    LOC_CPU
                } else if id.starts_with("/gpu") {
                    LOC_GPU
                } else {
                    LOC_OTHER
                };
                out.push(Reading::new("LHM", hardware, &node.text, kind, location, v, &units));
            }
        }
    }
    for child in &node.children {
        walk_lhm(child, depth + 1, hardware, kinds, out);
    }
}

pub fn read_lhm(agent: &ureq::Agent, port: u16, kinds: &[i32]) -> Result<Vec<Reading>, String> {
    let root: LhmNode = agent
        .get(&format!("http://127.0.0.1:{port}/data.json"))
        .call()
        .map_err(|_| format!("not running (enable Options > Remote Web Server, port {port})"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("unexpected data: {e}"))?;
    let mut out = Vec::new();
    walk_lhm(&root, 0, "", kinds, &mut out);
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// Bundled LibreHardwareMonitorLib helper (no GUI, HTTP server, or external LHM process).
// The helper is launched from the same trusted installation directory as aio-screen.exe.
// It writes line-delimited JSON. A dedicated reader thread keeps the sensor service's
// one-second loop from blocking on stdout or .NET hardware probing.

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProbeSnapshot {
    schema_version: u32,
    readings: Vec<ProbeSensor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ProbeSensor {
    id: String,
    hardware_name: String,
    hardware_type: String,
    name: String,
    #[serde(rename = "Type")]
    sensor_type: String,
    value: f64,
}

fn probe_reading(s: ProbeSensor) -> Option<Reading> {
    if !s.value.is_finite() || s.id.is_empty() { return None; }
    let (kind, unit) = match s.sensor_type.as_str() {
        "Temperature" => (1, "°C"),
        "Voltage" => (2, "V"),
        "Fan" => (3, "RPM"),
        "Current" => (4, "A"),
        "Power" => (5, "W"),
        "Clock" => (6, "MHz"),
        "Load" => (7, "%"),
        "Control" | "Level" => (8, "%"),
        "Data" => (8, "GB"),
        "SmallData" => (8, "MB"),
        "Throughput" => (8, "B/s"),
        "Factor" | "Timing" => (8, ""),
        _ => return None,
    };
    let location = match s.hardware_type.as_str() {
        "Cpu" => LOC_CPU,
        "GpuNvidia" | "GpuAmd" | "GpuIntel" => LOC_GPU,
        _ => LOC_OTHER,
    };
    // Unlike the external LHM server's display-name key, internal keys are based on
    // sensor.Identifier. Renames and localization do not invalidate saved widgets.
    Some(Reading {
        key: format!("Internal LHM · {}", s.id),
        sensor: format!("Internal LHM · {}", s.hardware_name.trim_matches(char::from(0))),
        name: s.name,
        kind,
        sensor_location: location,
        value: s.value,
        units: unit.to_string(),
    })
}

struct InternalProbe {
    child: Child,
    reader: Option<JoinHandle<()>>,
    latest: Arc<Mutex<Option<(Instant, Vec<Reading>)>>>,
}

impl InternalProbe {
    fn start() -> Result<Self, String> {
        let path = std::env::current_exe()
            .map_err(|e| format!("cannot locate installation: {e}"))?
            .parent()
            .ok_or("cannot locate application directory")?
            .join("SensorProbe.exe");
        if !path.is_file() {
            return Err("not bundled (SensorProbe.exe missing)".into());
        }
        let mut child = Command::new(&path)
            .arg("--interval-ms").arg("1000")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("cannot launch helper: {e}"))?;
        let stdout = child.stdout.take().ok_or("cannot capture helper output")?;
        let latest = Arc::new(Mutex::new(None));
        let shared = Arc::clone(&latest);
        let reader = std::thread::spawn(move || {
            // Keep only the latest complete snapshot, never an incomplete JSON line.
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(snapshot) = serde_json::from_str::<ProbeSnapshot>(&line) else { continue };
                if snapshot.schema_version != 1 { continue; }
                let parsed = snapshot.readings.into_iter().filter_map(probe_reading).collect();
                if let Ok(mut current) = shared.lock() {
                    *current = Some((Instant::now(), parsed));
                }
            }
        });
        Ok(Self { child, reader: Some(reader), latest })
    }

    fn read(&mut self, kinds: &[i32]) -> Result<Vec<Reading>, String> {
        match self.child.try_wait() {
            Ok(Some(status)) => return Err(format!("helper exited ({status})")),
            Err(e) => return Err(format!("helper process error: {e}")),
            Ok(None) => {}
        }
        let data = self.latest.lock().map_err(|_| "helper reader lock error")?;
        match data.as_ref() {
            Some((timestamp, readings)) if timestamp.elapsed() <= Duration::from_secs(5) =>
                Ok(readings.iter().filter(|r| kinds.is_empty() || kinds.contains(&r.kind)).cloned().collect()),
            Some(_) => Err("helper data is stale".into()),
            None => Err("waiting for first sensor snapshot".into()),
        }
    }
}

impl Drop for InternalProbe {
    fn drop(&mut self) {
        // Closing the service kills the helper; it must never be orphaned.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() { let _ = reader.join(); }
    }
}

// ---------------------------------------------------------------------------------------
// NVML

/// NVIDIA GPUs through NVML (`nvml.dll`, installed with the NVIDIA driver).
pub struct Nvml {
    _lib: Library,
    devices: Vec<(usize, String)>,
    shutdown: unsafe extern "C" fn() -> i32,
    temperature: unsafe extern "C" fn(usize, i32, *mut u32) -> i32,
    utilization: unsafe extern "C" fn(usize, *mut [u32; 2]) -> i32,
}

impl Nvml {
    pub fn load() -> Result<Self, String> {
        // SAFETY: NVML's stable C API; nvmlDevice_t is an opaque pointer-sized handle.
        unsafe {
            let lib = Library::new("nvml.dll").map_err(|_| "no NVIDIA driver".to_string())?;
            macro_rules! sym {
                ($name:literal, $t:ty) => {
                    *lib.get::<$t>(concat!($name, "\0").as_bytes()).map_err(|e| format!("NVML {}: {e}", $name))?
                };
            }
            let init = sym!("nvmlInit_v2", unsafe extern "C" fn() -> i32);
            let count = sym!("nvmlDeviceGetCount_v2", unsafe extern "C" fn(*mut u32) -> i32);
            let handle = sym!("nvmlDeviceGetHandleByIndex_v2", unsafe extern "C" fn(u32, *mut usize) -> i32);
            let name = sym!("nvmlDeviceGetName", unsafe extern "C" fn(usize, *mut c_char, u32) -> i32);
            let shutdown = sym!("nvmlShutdown", unsafe extern "C" fn() -> i32);
            let temperature = sym!("nvmlDeviceGetTemperature", unsafe extern "C" fn(usize, i32, *mut u32) -> i32);
            let utilization = sym!("nvmlDeviceGetUtilizationRates", unsafe extern "C" fn(usize, *mut [u32; 2]) -> i32);
            let rc = init();
            if rc != 0 {
                return Err(format!("nvmlInit failed ({rc})"));
            }
            let mut n = 0u32;
            count(&mut n);
            let mut devices = Vec::new();
            for i in 0..n {
                let mut h = 0usize;
                if handle(i, &mut h) == 0 {
                    let mut buf = [0u8; 96];
                    name(h, buf.as_mut_ptr() as *mut c_char, buf.len() as u32);
                    devices.push((h, c_str(&buf)));
                }
            }
            Ok(Nvml { _lib: lib, devices, shutdown, temperature, utilization })
        }
    }

    pub fn read(&self) -> Vec<Reading> {
        let mut out = Vec::new();
        for (h, name) in &self.devices {
            // SAFETY: valid device handles from nvmlDeviceGetHandleByIndex.
            unsafe {
                let mut t = 0u32;
                if (self.temperature)(*h, 0, &mut t) == 0 {
                    out.push(Reading::new("NVML", name, "GPU Temperature", READING_TEMP, LOC_GPU, t as f64, "°C"));
                }
                let mut u = [0u32; 2];
                if (self.utilization)(*h, &mut u) == 0 {
                    out.push(Reading::new("NVML", name, "GPU Core Load", READING_USAGE, LOC_GPU, u[0] as f64, "%"));
                    out.push(Reading::new("NVML", name, "GPU Memory Controller Load", READING_USAGE, LOC_GPU, u[1] as f64, "%"));
                }
            }
        }
        out
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        // SAFETY: matching nvmlShutdown for a successful nvmlInit.
        unsafe {
            (self.shutdown)();
        }
    }
}

// ---------------------------------------------------------------------------------------
// All sources together

pub struct SensorHub {
    agent: ureq::Agent,
    cpu_temp: Option<CpuTemp>,
    cpu_temp_status: String,
    cpu_load: CpuLoad,
    cpu_name: String,
    nvml: Option<Nvml>,
    nvml_status: String,
    next_driver_try: Instant,
    internal_probe: Option<InternalProbe>,
    next_probe_try: Instant,
}

impl Default for SensorHub {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(800)))
            .build()
            .into();
        let mut cpu_load = CpuLoad::default();
        cpu_load.read(); // first sample, so the next read already has a delta
        SensorHub {
            agent,
            cpu_temp: None,
            cpu_temp_status: String::new(),
            cpu_load,
            cpu_name: crate::cpu::cpu_info().name,
            nvml: None,
            nvml_status: String::new(),
            next_driver_try: Instant::now(),
            internal_probe: None,
            next_probe_try: Instant::now(),
        }
    }
}

impl SensorHub {
    /// Poll every source. Returns the readings and a one-line summary per source.
    pub fn read(&mut self, lhm_port: u16, kinds: &[i32]) -> (Vec<Reading>, String) {
        if (self.nvml.is_none() || self.cpu_temp.is_none()) && Instant::now() >= self.next_driver_try {
            if self.cpu_temp.is_none() {
                match CpuTemp::load() {
                    Ok(t) => self.cpu_temp = Some(t),
                    Err(e) => self.cpu_temp_status = e,
                }
            }
            if self.nvml.is_none() {
                match Nvml::load() {
                    Ok(n) => self.nvml = Some(n),
                    Err(e) => self.nvml_status = e,
                }
            }
            self.next_driver_try = Instant::now() + NVML_RETRY;
        }
        let mut readings = Vec::new();
        let mut summary = Vec::new();

        let want = |k: i32| kinds.is_empty() || kinds.contains(&k);
        match &self.cpu_temp {
            Some(t) => {
                if let Some((v, name)) = t.read().filter(|_| want(READING_TEMP)) {
                    readings.push(Reading::new("Built-in", t.cpu_name(), name, READING_TEMP, LOC_CPU, v, "°C"));
                }
                summary.push(format!("Built-in CPU temperature: ok ({})", t.cpu_name()));
            }
            None => summary.push(format!("Built-in CPU temperature: {}", self.cpu_temp_status)),
        }
        if let Some(load) = self.cpu_load.read().filter(|_| want(READING_USAGE)) {
            readings.push(Reading::new("Built-in", &self.cpu_name, "Total CPU Usage", READING_USAGE, LOC_CPU, load, "%"));
        }

        // Internal readings are an independent source; external LHM, HWiNFO and
        // built-in sensors remain available and their saved keys are unchanged.
        if self.internal_probe.is_none() && Instant::now() >= self.next_probe_try {
            match InternalProbe::start() {
                Ok(p) => self.internal_probe = Some(p),
                Err(e) => {
                    summary.push(format!("Internal LHM: {e}"));
                    self.next_probe_try = Instant::now() + Duration::from_secs(30);
                }
            }
        }
        if let Some(probe) = self.internal_probe.as_mut() {
            match probe.read(kinds) {
                Ok(r) => {
                    summary.push(format!("Internal LHM: ok ({} readings)", r.len()));
                    readings.extend(r);
                }
                Err(e) => {
                    // Exit means a one-off restart. Staleness may be transient while
                    // the helper is enumerating hardware, so keep it running.
                    let exited = e.starts_with("helper exited") || e.starts_with("helper process error");
                    summary.push(format!("Internal LHM: {e}"));
                    if exited {
                        self.internal_probe = None;
                        self.next_probe_try = Instant::now() + Duration::from_secs(10);
                    }
                }
            }
        }

        for (name, result) in [("HWiNFO", read_hwinfo(kinds)), ("LibreHardwareMonitor", read_lhm(&self.agent, lhm_port, kinds))] {
            match result {
                Ok(r) => {
                    summary.push(format!("{name}: ok ({} readings)", r.len()));
                    readings.extend(r);
                }
                Err(e) => summary.push(format!("{name}: {e}")),
            }
        }
        match &self.nvml {
            Some(n) => {
                let r = n.read();
                summary.push(format!("NVML: ok ({} GPU)", n.devices.len()));
                readings.extend(r);
            }
            None => summary.push(format!("NVML: {}", self.nvml_status)),
        }
        (readings, summary.join("\n"))
    }
}

pub fn kind_name(kind: i32) -> &'static str {
    READING_TYPES.iter().find(|(k, _)| *k == kind).map_or("?", |(_, n)| n)
}

// ---------------------------------------------------------------------------------------
// Stats

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stat {
    CpuTemp,
    CpuLoad,
    GpuTemp,
    GpuLoad,
}

impl Stat {
    pub const ALL: [Stat; 4] = [Stat::CpuTemp, Stat::CpuLoad, Stat::GpuTemp, Stat::GpuLoad];

    pub fn label(self) -> &'static str {
        match self {
            Stat::CpuTemp | Stat::CpuLoad => "CPU",
            Stat::GpuTemp | Stat::GpuLoad => "GPU",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Stat::CpuTemp => "CPU temperature",
            Stat::CpuLoad => "CPU load",
            Stat::GpuTemp => "GPU temperature",
            Stat::GpuLoad => "GPU load",
        }
    }

    pub fn is_temp(self) -> bool {
        matches!(self, Stat::CpuTemp | Stat::GpuTemp)
    }

    pub fn reading_kind(self) -> i32 {
        if self.is_temp() {
            READING_TEMP
        } else {
            READING_USAGE
        }
    }

    /// Heuristic default reading for this stat, across HWiNFO, LHM and NVML naming.
    pub fn auto_pick(self, readings: &[Reading]) -> Option<&Reading> {
        let score = |r: &Reading| -> i32 {
            if r.kind != self.reading_kind() {
                return -1;
            }
            let n = r.name.to_ascii_lowercase();
            let s = r.sensor.to_ascii_lowercase();
            // Prefer our own readings so the stats don't change when HWiNFO/LHM start or stop.
            let builtin = if s.starts_with("built-in") { 5 } else { 0 };
            match self {
                Stat::CpuTemp if r.sensor_location == LOC_CPU => {
                    builtin
                        + if n.contains("tctl/tdie") {
                            100
                        } else if n.contains("package") {
                            90
                        } else if n.contains("tdie") || n.contains("tctl") {
                            80
                        } else if n.contains("core") && n.contains("max") {
                            70
                        } else {
                            10
                        }
                }
                Stat::CpuLoad if r.sensor_location == LOC_CPU => {
                    if n.contains("total cpu usage") || n == "cpu total" {
                        100 + builtin
                    } else {
                        0
                    }
                }
                Stat::GpuTemp | Stat::GpuLoad if r.sensor_location == LOC_GPU => {
                    // Prefer the discrete card over integrated graphics, and NVML for NVIDIA.
                    let discrete = s.contains("nvidia") || s.contains("geforce") || s.contains(" rx ");
                    let base = if discrete { 50 } else { 0 } + if s.starts_with("nvml") { 5 } else { 0 };
                    let wanted: &[&str] =
                        if self == Stat::GpuTemp { &["gpu temperature", "gpu core"] } else { &["gpu core load", "gpu core", "gpu utilization"] };
                    if wanted.contains(&n.as_str()) {
                        base + 40
                    } else if n.starts_with("gpu") && !n.contains("memory") && !n.contains("hot") {
                        base + 10
                    } else {
                        base / 2
                    }
                }
                _ => -1,
            }
        };
        readings.iter().filter(|r| score(r) > 0).max_by_key(|r| score(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hwinfo_shared_memory() {
        // Header + 1 sensor (264 bytes) + 1 reading (316 bytes), HWiNFO SM2 layout.
        let (s_off, s_size, r_off, r_size) = (44usize, 264usize, 44 + 264, 316usize);
        let mut d = vec![0u8; r_off + r_size];
        d[0..4].copy_from_slice(&HWINFO_SIGNATURE.to_le_bytes());
        for (off, v) in [(20, s_off), (24, s_size), (28, 1), (32, r_off), (36, r_size), (40, 1)] {
            d[off..off + 4].copy_from_slice(&(v as u32).to_le_bytes());
        }
        let name = b"CPU [#0]: AMD Ryzen 7 9700X: Enhanced";
        d[s_off + 8..s_off + 8 + name.len()].copy_from_slice(name);
        d[r_off..r_off + 4].copy_from_slice(&1u32.to_le_bytes());
        let label = b"CPU (Tctl/Tdie)";
        d[r_off + 12..r_off + 12 + label.len()].copy_from_slice(label);
        d[r_off + 268] = b'C';
        d[r_off + 284..r_off + 292].copy_from_slice(&61.5f64.to_le_bytes());
        let r = parse_hwinfo(&d, &[]).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].sensor_location, LOC_CPU);
        assert_eq!(r[0].value, 61.5);
        assert_eq!(Stat::CpuTemp.auto_pick(&r).unwrap().name, "CPU (Tctl/Tdie)");
    }

    #[test]
    fn parses_lhm_tree_and_picks_stats() {
        let json = r#"{"Text":"Sensor","Children":[{"Text":"PC","Children":[
            {"Text":"AMD Ryzen 7 9700X","Children":[
                {"Text":"Temperatures","Children":[{"Text":"Core (Tctl/Tdie)","Value":"58,3 °C","Type":"Temperature","SensorId":"/amdcpu/0/temperature/2","Children":[]}]},
                {"Text":"Load","Children":[{"Text":"CPU Total","Value":"12.5 %","Type":"Load","SensorId":"/amdcpu/0/load/0","Children":[]}]}]},
            {"Text":"NVIDIA GeForce RTX 5070","Children":[
                {"Text":"Temperatures","Children":[{"Text":"GPU Core","Value":"47.0 °C","Type":"Temperature","SensorId":"/gpu-nvidia/0/temperature/0","Children":[]}]}]}
        ]}]}"#;
        let root: LhmNode = serde_json::from_str(json).unwrap();
        let mut r = Vec::new();
        walk_lhm(&root, 0, "", &[], &mut r);
        assert_eq!(r.len(), 3);
        assert_eq!(Stat::CpuTemp.auto_pick(&r).unwrap().value, 58.3);
        assert_eq!(Stat::CpuLoad.auto_pick(&r).unwrap().value, 12.5);
        let gpu = Stat::GpuTemp.auto_pick(&r).unwrap();
        assert_eq!(gpu.sensor, "LHM · NVIDIA GeForce RTX 5070");
        assert_eq!(gpu.value, 47.0);
    }
}
