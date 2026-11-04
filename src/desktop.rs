//! Per-user desktop integration for the GUI: tray icon, sign-in startup entry, preferences,
//! and making sure only one copy of the app runs.

use std::cell::Cell;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE};

use crate::icon;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "AIO Cooler Screen";
const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const INSTANCE_MUTEX: &str = r"Local\AioCoolerScreenGui";
const SHOW_EVENT: &str = r"Local\AioCoolerScreenShow";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------------------------------
// Preferences (per user)

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Show the tray icon; closing the window then keeps the app in the tray.
    pub show_tray: bool,
    /// 0 = English (default), 1 = Türkçe.
    pub language: i32,
    /// 0 = square (default), 1 = round preview. Does not change LCD output.
    pub preview_shape: i32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { show_tray: true, language: 0, preview_shape: 0 }
    }
}

fn prefs_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("AIO Screen").join("gui.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        std::fs::read_to_string(prefs_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = prefs_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------------------
// Open in the tray at sign-in (HKCU Run entry)

pub fn open_at_login() -> bool {
    let mut size = 0u32;
    // SAFETY: size query only; valid NUL-terminated strings.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_VALUE).as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        ) == 0
    }
}

pub fn set_open_at_login(on: bool) -> Result<(), String> {
    // SAFETY: valid NUL-terminated strings and a correctly sized REG_SZ buffer.
    let rc = unsafe {
        if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd = wide(&format!("\"{}\" --tray", exe.display()));
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                wide(RUN_VALUE).as_ptr(),
                REG_SZ,
                cmd.as_ptr().cast(),
                (cmd.len() * 2) as u32,
            )
        } else {
            let rc = RegDeleteKeyValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(RUN_VALUE).as_ptr());
            if rc == 2 { 0 } else { rc } // ERROR_FILE_NOT_FOUND: already off
        }
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(format!("could not change the sign-in setting (error {rc})"))
    }
}

// ---------------------------------------------------------------------------------------
// Taskbar theme

/// True when the taskbar uses the light theme (so the tray icon should be dark).
pub fn taskbar_is_light() -> bool {
    let mut value = 0u32;
    let mut size = 4u32;
    // SAFETY: reading a REG_DWORD into a 4-byte buffer.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(PERSONALIZE_KEY).as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    rc == 0 && value != 0
}

// ---------------------------------------------------------------------------------------
// Single instance

pub enum Instance {
    /// This is the only copy; keep the mutex for the life of the process.
    First(HANDLE),
    /// Another copy is running and has been asked to show its window.
    AlreadyRunning,
}

pub fn claim_instance() -> Instance {
    // SAFETY: plain Win32 object creation with NUL-terminated names.
    unsafe {
        let mutex = CreateMutexW(std::ptr::null(), 0, wide(INSTANCE_MUTEX).as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let ev = OpenEventW(EVENT_MODIFY_STATE, 0, wide(SHOW_EVENT).as_ptr());
            if !ev.is_null() {
                SetEvent(ev);
                CloseHandle(ev);
            }
            return Instance::AlreadyRunning;
        }
        Instance::First(mutex)
    }
}

/// Call `on_show` (from a background thread) whenever another launch asks us to show.
pub fn listen_for_show(on_show: impl Fn() + Send + 'static) {
    // SAFETY: auto-reset event owned by the listener thread for the life of the process.
    let ev = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT).as_ptr()) } as usize;
    if ev == 0 {
        return;
    }
    std::thread::spawn(move || loop {
        // SAFETY: valid event handle.
        unsafe { WaitForSingleObject(ev as HANDLE, INFINITE) };
        on_show();
    });
}

// ---------------------------------------------------------------------------------------
// Tray icon

pub enum TrayAction {
    Open,
    ToggleStats(bool),
    ToggleDisplay,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    light: bool,
    open: MenuItem,
    stats: CheckMenuItem,
    display: MenuItem,
    quit: MenuItem,
    language: Cell<i32>,
    display_off: Cell<bool>,
}

fn display_label(off: bool, language: i32) -> &'static str {
    match (language == 1, off) {
        (true, true) => "Ekranı aç",
        (true, false) => "Ekranı kapat",
        (false, true) => "Turn on display",
        (false, false) => "Turn off display",
    }
}

fn mono_icon(light_taskbar: bool) -> Option<Icon> {
    let rgb = if light_taskbar { [24, 24, 24] } else { [255, 255, 255] };
    let img = icon::render_mono(32, rgb);
    Icon::from_rgba(img.into_raw(), 32, 32).ok()
}

impl Tray {
    pub fn new(stats_on: bool, display_off: bool, language: i32) -> Result<Tray, String> {
        let turkish = language == 1;
        let open = MenuItem::new(if turkish { "AIO Cooler Screen'i aç" } else { "Open AIO Cooler Screen" }, true, None);
        let stats = CheckMenuItem::new(if turkish { "Canlı verileri göster" } else { "Show live stats" }, true, stats_on, None);
        let display = MenuItem::new(display_label(display_off, language), true, None);
        let quit = MenuItem::new(if turkish { "Çıkış" } else { "Quit" }, true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &stats, &display, &PredefinedMenuItem::separator(), &quit]).map_err(|e| e.to_string())?;
        let light = taskbar_is_light();
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("AIO Cooler Screen");
        if let Some(icon) = mono_icon(light) {
            builder = builder.with_icon(icon);
        }
        let icon = builder.build().map_err(|e| e.to_string())?;
        Ok(Tray { icon, light, open, stats, display, quit, language: Cell::new(language), display_off: Cell::new(display_off) })
    }

    /// Follow the Windows theme so the monochrome icon stays visible.
    pub fn refresh_theme(&mut self) {
        let light = taskbar_is_light();
        if light != self.light {
            self.light = light;
            let _ = self.icon.set_icon(mono_icon(light));
        }
    }

    pub fn set_tooltip(&self, text: &str) {
        let _ = self.icon.set_tooltip(Some(text));
    }

    pub fn set_stats_checked(&self, on: bool) {
        self.stats.set_checked(on);
    }

    pub fn set_display_off(&self, off: bool) {
        self.display.set_text(display_label(off, self.language.get()));
        self.display_off.set(off);
    }

    /// Update the current menu items when the user switches application language.
    /// IDs and checked state stay unchanged; only visible text is localized.
    pub fn set_language(&self, language: i32) {
        let turkish = language == 1;
        self.open.set_text(if turkish { "AIO Cooler Screen'i aç" } else { "Open AIO Cooler Screen" });
        self.stats.set_text(if turkish { "Canlı verileri göster" } else { "Show live stats" });
        self.display.set_text(display_label(self.display_off.get(), language));
        self.quit.set_text(if turkish { "Çıkış" } else { "Quit" });
        self.language.set(language);
    }

    /// Pending clicks and menu choices.
    pub fn poll(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            match ev {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => actions.push(TrayAction::Open),
                _ => {}
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == *self.open.id() {
                actions.push(TrayAction::Open);
            } else if ev.id == *self.stats.id() {
                actions.push(TrayAction::ToggleStats(self.stats.is_checked()));
            } else if ev.id == *self.display.id() {
                actions.push(TrayAction::ToggleDisplay);
            } else if ev.id == *self.quit.id() {
                actions.push(TrayAction::Quit);
            }
        }
        actions
    }
}
