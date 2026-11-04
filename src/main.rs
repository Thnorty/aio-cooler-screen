#![cfg_attr(not(test), windows_subsystem = "windows")]

mod config;
mod cpu;
mod desktop;
mod device;
mod ffmpeg;
mod gui;
mod i18n;
mod icon;
mod media;
mod overlay;
mod protocol;
mod sensors;
mod service;

use std::process::ExitCode;

/// Write every sensor reading (and the automatic picks) to a text file. Diagnostic aid.
fn dump_sensors(out: &str) -> Result<(), String> {
    use std::fmt::Write as _;
    let cfg = config::Config::load();
    let mut hub = sensors::SensorHub::default();
    std::thread::sleep(std::time::Duration::from_millis(500)); // let CPU load accumulate a delta
    let (readings, summary) = hub.read(cfg.effective_lhm_port(), &[]);
    let mut s = format!("{summary}\n{} readings\n", readings.len());
    for stat in sensors::Stat::ALL {
        let pick = stat.auto_pick(&readings).map(|r| format!("{} = {:.1} {}", r.key, r.value, r.units));
        let _ = writeln!(s, "AUTO {:?}: {}", stat, pick.unwrap_or_else(|| "-".into()));
    }
    for r in &readings {
        let _ = writeln!(s, "{}\t{}\tloc={}\t{:.2} {}", sensors::kind_name(r.kind), r.key, r.sensor_location, r.value, r.units);
    }
    std::fs::write(out, s).map_err(|e| e.to_string())
}

/// Report a command's result through an optional file (the process has no console).
fn report(result: Result<(), String>, file: Option<&String>) -> ExitCode {
    let ok = result.is_ok();
    if let Some(f) = file {
        let _ = std::fs::write(f, result.err().unwrap_or_default());
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let out = args.get(2).filter(|a| !a.starts_with("--"));
    match args.get(1).map(String::as_str) {
        Some("service") => report(service::run_dispatcher(), None),
        Some("install") => report(service::install(), out),
        // The installer's uninstaller passes --keep-files and deletes the files itself.
        Some("uninstall") => {
            let _ = desktop::set_open_at_login(false);
            report(service::uninstall(!flag("--keep-files")), out)
        }
        Some("restart") => report(service::restart(), out),
        Some("set-autostart") => {
            let on = args.get(2).map(String::as_str) != Some("off");
            report(service::set_autostart(on), args.get(3))
        }
        Some("fetch-ffmpeg") => report(ffmpeg::fetch_command(out), out),
        Some("make-icon") => report(icon::write_assets(std::path::Path::new(out.map_or("assets", |s| s.as_str()))), None),
        Some("dump-sensors") => {
            let path = out.cloned().unwrap_or_else(|| "sensors.txt".into());
            let r = dump_sensors(&path);
            if let Err(e) = &r {
                let _ = std::fs::write(&path, format!("error: {e}\n"));
            }
            report(r, None)
        }
        _ => report(gui::run(flag("--tray")), None),
    }
}
