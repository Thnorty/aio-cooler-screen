//! The Windows service. Runs as LocalSystem from boot (no UAC prompt, and it can open
//! HWiNFO's Global shared memory), owns the cooler connection, uploads the configured media,
//! and refreshes the stats overlay every second, which also keeps the screen from blanking.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use hidapi::HidApi;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo,
    ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use crate::config::{self, Config, Status, SERVICE_DISPLAY_NAME, SERVICE_NAME};
use crate::device::Screen;
use crate::media::{encode_png, now_ms, timestamp_name, SCREEN_SIZE};
use crate::overlay;
use crate::protocol::MediaKind;
use crate::sensors::{Reading, SensorHub, READING_TEMP, READING_USAGE};

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;
/// After this long without a successful update the screen may have gone dark (e.g. the
/// PC slept), so the connect handshake is repeated to wake it.
const REWAKE_AFTER: Duration = Duration::from_secs(20);
const LOG_MAX_BYTES: u64 = 1_000_000;

define_windows_service!(ffi_service_main, service_main);

pub fn run_dispatcher() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| format!("service dispatcher: {e}"))
}

fn log(msg: &str) {
    use std::io::Write as _;
    let path = config::data_dir().join("service.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {msg}", timestamp_name("").trim_end_matches('.'));
    }
}

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run() {
        log(&format!("service failed: {e}"));
    }
}

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}

fn run() -> windows_service::Result<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let handle = service_control_handler::register(SERVICE_NAME, handler)?;
    handle.set_service_status(status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
    ))?;
    log("service started");
    worker(&stop_rx);
    log("service stopped");
    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    Ok(())
}

#[derive(Default)]
struct SensorState {
    readings: Vec<Reading>,
    summary: String,
}

/// Poll HWiNFO, LibreHardwareMonitor and NVML once a second.
fn spawn_sensor_thread(state: Arc<Mutex<SensorState>>, stop: Arc<AtomicBool>, lhm_port: Arc<AtomicU16>) {
    std::thread::spawn(move || {
        let mut hub = SensorHub::default();
        while !stop.load(Ordering::Relaxed) {
            let (readings, summary) = hub.read(lhm_port.load(Ordering::Relaxed), &[]);
            if let Ok(mut s) = state.lock() {
                *s = SensorState { readings, summary };
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

/// Media file named in the config, constrained to a plain file inside the data dir.
fn media_path(cfg: &Config) -> Option<std::path::PathBuf> {
    let name = cfg.media_file.as_deref()?;
    (Path::new(name).file_name()? == name).then(|| config::data_dir().join(name))
}

/// Darken the screen and ask it to switch off. The opaque black overlay makes it go dark
/// at once even if the screen ignores the power command.
fn blank(s: &mut Screen) -> Result<(), String> {
    let black = image::RgbaImage::from_pixel(SCREEN_SIZE, SCREEN_SIZE, image::Rgba([0, 0, 0, 255]));
    s.upload(&encode_png(&black)?, &timestamp_name("osd"), MediaKind::Overlay)?;
    match s.power_off() {
        Ok(()) => log("display turned off"),
        Err(e) => log(&format!("display blanked; power off not accepted ({e}), it switches off after its timeout")),
    }
    Ok(())
}

fn worker(stop_rx: &mpsc::Receiver<()>) {
    let sensors = Arc::new(Mutex::new(SensorState::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let mut cfg = Config::load();
    let mut cfg_mtime = Config::modified();
    let lhm_port = Arc::new(AtomicU16::new(cfg.effective_lhm_port()));
    spawn_sensor_thread(sensors.clone(), stop.clone(), lhm_port.clone());

    let mut api: Option<HidApi> = None;
    let mut screen: Option<Screen> = None;
    let mut applied: Option<u64> = None;
    let mut last_error: Option<String> = None;
    let mut last_ok = Instant::now();
    // The open connection has been blanked because the user turned the display off.
    let mut blanked = false;

    loop {
        match stop_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        let mtime = Config::modified();
        if mtime != cfg_mtime {
            cfg = Config::load();
            cfg_mtime = mtime;
            lhm_port.store(cfg.effective_lhm_port(), Ordering::Relaxed);
        }

        if screen.is_none() {
            match api.as_mut() {
                Some(a) => {
                    let _ = a.refresh_devices();
                }
                None => api = HidApi::new().map_err(|e| log(&format!("HID init failed: {e}"))).ok(),
            }
            screen = api.as_ref().and_then(|a| match Screen::open(a) {
                Ok(s) => {
                    log("cooler connected");
                    last_ok = Instant::now();
                    Some(s)
                }
                Err(e) => {
                    if last_error.as_deref() != Some(e.as_str()) {
                        log(&format!("cannot connect: {e}"));
                    }
                    last_error = Some(e);
                    None
                }
            });
        }

        let (readings, sensor_summary) = match sensors.lock() {
            Ok(s) => (s.readings.clone(), s.summary.clone()),
            Err(_) => (Vec::new(), "sensor thread failed".into()),
        };

        if let Some(s) = screen.as_mut() {
            let result = (|| -> Result<(), String> {
                if cfg.display_off {
                    // Blank once, then send nothing so the screen stays off.
                    if !blanked {
                        blank(s)?;
                        blanked = true;
                    }
                    return Ok(());
                }
                if blanked {
                    s.wake()?;
                    blanked = false;
                    log("display turned on");
                } else if last_ok.elapsed() > REWAKE_AFTER {
                    s.handshake()?;
                    log("re-sent connect handshake after a gap");
                }
                if applied != Some(cfg.media_version) {
                    if let Some(path) = media_path(&cfg) {
                        // A missing or unreadable file is a settings problem, not a device
                        // error: log it and keep showing the stats.
                        match std::fs::read(&path) {
                            Ok(data) => {
                                let (ext, kind) = if cfg.media_is_video { ("mp4", MediaKind::Video) } else { ("png", MediaKind::Image) };
                                s.upload(&data, &timestamp_name(ext), kind)?;
                                log(&format!("uploaded {} ({} bytes)", path.display(), data.len()));
                            }
                            Err(e) => log(&format!("skipping background {}: {e}", path.display())),
                        }
                    }
                    applied = Some(cfg.media_version);
                }
                let values = cfg.stats.resolve(&readings);
                let png = encode_png(&overlay::render(&cfg.stats, &values, chrono::Local::now()))?;
                s.upload(&png, &timestamp_name("osd"), MediaKind::Overlay)
            })();
            match result {
                Ok(()) => {
                    last_error = None;
                    last_ok = Instant::now();
                }
                Err(e) => {
                    if last_error.as_deref() != Some(e.as_str()) {
                        log(&e);
                    }
                    last_error = Some(e);
                    screen = None; // reconnect on the next tick
                    blanked = false; // a new connection wakes the screen, so blank it again
                }
            }
        }

        let st = Status {
            updated_ms: now_ms(),
            device_connected: screen.is_some(),
            sensors: sensor_summary,
            media_version_applied: applied,
            last_error: last_error.clone(),
            readings,
        };
        let _ = st.save();
    }
    stop.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------
// Install / uninstall (run elevated, launched by the GUI).

fn wait_for_state(svc: &windows_service::service::Service, state: ServiceState) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if svc.query_status().is_ok_and(|s| s.current_state == state) {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn copy_with_retry(from: &Path, to: &Path) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..20 {
        match std::fs::copy(from, to) {
            Ok(_) => return Ok(()),
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("cannot copy to {}: {last}", to.display()))
}

/// Install or update the service: copy this executable to Program Files, open the data
/// dir to standard users (config and media), register the service and start it.
pub fn install() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = config::install_dir();
    let target = dir.join("aio-screen.exe");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .map_err(|e| format!("cannot open service manager (run as admin): {e}"))?;
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::START | ServiceAccess::STOP | ServiceAccess::CHANGE_CONFIG;
    let existing = mgr.open_service(SERVICE_NAME, access).ok();
    if let Some(svc) = &existing {
        if svc.query_status().is_ok_and(|s| s.current_state != ServiceState::Stopped) {
            let _ = svc.stop();
            wait_for_state(svc, ServiceState::Stopped);
        }
    }
    if exe != target {
        copy_with_retry(&exe, &target)?;
    }

    let data = config::data_dir();
    std::fs::create_dir_all(&data).map_err(|e| format!("cannot create {}: {e}", data.display()))?;
    // BUILTIN\Users get modify on the data dir so the GUI can write config and media.
    let _ = Command::new("icacls").arg(&data).args(["/grant", "*S-1-5-32-545:(OI)(CI)M"]).output();

    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY_NAME.into(),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target,
        launch_arguments: vec!["service".into()],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let svc = match existing {
        Some(svc) => {
            svc.change_config(&info).map_err(|e| format!("cannot update service: {e}"))?;
            svc
        }
        None => mgr.create_service(&info, access).map_err(|e| format!("cannot create service: {e}"))?,
    };
    let _ = svc.set_description("Shows media and live CPU/GPU stats on the Monster AIO Cooler screen.");
    svc.start::<&str>(&[]).map_err(|e| format!("cannot start service: {e}"))?;
    Ok(())
}

pub fn uninstall(remove_files: bool) -> Result<(), String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("cannot open service manager (run as admin): {e}"))?;
    let svc = mgr
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE)
        .map_err(|e| format!("service not installed: {e}"))?;
    if svc.query_status().is_ok_and(|s| s.current_state != ServiceState::Stopped) {
        let _ = svc.stop();
        wait_for_state(&svc, ServiceState::Stopped);
    }
    svc.delete().map_err(|e| format!("cannot delete service: {e}"))?;
    drop(svc);
    if remove_files {
        let _ = std::fs::remove_dir_all(config::install_dir());
    }
    Ok(())
}

/// Whether the service starts with Windows (Automatic) or only on demand (Manual).
pub fn set_autostart(on: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let out = Command::new("sc.exe")
        .args(["config", SERVICE_NAME, "start=", if on { "auto" } else { "demand" }])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("cannot run sc.exe: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("could not change the startup type: {}", String::from_utf8_lossy(&out.stdout).trim()))
    }
}

/// `Some(true)` when the service starts with Windows. Works without admin.
pub fn query_autostart() -> Option<bool> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    let svc = mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_CONFIG).ok()?;
    svc.query_config().ok().map(|c| c.start_type == ServiceStartType::AutoStart)
}

/// Stop the service (if running) and start it again, keeping its configuration.
pub fn restart() -> Result<(), String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("cannot open service manager (run as admin): {e}"))?;
    let svc = mgr
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START)
        .map_err(|e| format!("service not installed: {e}"))?;
    if svc.query_status().is_ok_and(|s| s.current_state != ServiceState::Stopped) {
        let _ = svc.stop();
        wait_for_state(&svc, ServiceState::Stopped);
    }
    svc.start::<&str>(&[]).map_err(|e| format!("cannot start service: {e}"))
}

/// Current service state, or `None` when it is not installed. Works without admin.
pub fn query_state() -> Option<ServiceState> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    let svc = mgr.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).ok()?;
    svc.query_status().ok().map(|s| s.current_state)
}
