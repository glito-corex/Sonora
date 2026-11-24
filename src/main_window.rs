// -- main_window.rs -- //   (équivalent de gui.pyw)
//
// Fenêtre principale 900x520 : tout est dessiné en GDI dans une seule fenêtre
// (aucun contrôle enfant, aucune bibliothèque graphique) pour un coût mémoire minimal.
// Les couleurs, tailles, textes et la disposition reproduisent l'original.

use std::cell::{Cell, RefCell};
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EndPaint,
    InvalidateRect, SelectObject, SetWindowOrgEx, HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, GetSystemMetrics, GetWindowLongPtrW, LoadCursorW,
    PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_ARROW, MINMAXINFO,
    SIZE_MINIMIZED, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SW_SHOW, WM_APP,
    WM_APPCOMMAND, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_HOTKEY, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SIZE, WNDCLASSEXW,
    WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_THICKFRAME,
};

use crate::media_keys;
use crate::memory_trimming;
use crate::media_selection;
use crate::playback_commands::{self, PlaybackCommand};
use crate::player_state::PlayerState;
use crate::win32_dialogs;
use crate::window_layout;
use crate::win32_support::*;
use crate::{browser_window, playlist_navigation, seek_controls, volume_control};

const WM_MOUSELEAVE: u32 = 0x02A3;
const CLASS_NAME: &str = "SonoraMainWindow";
const WINDOW_TITLE: &str = "Sonora";

use window_layout::{MAX_CLIENT_HEIGHT, MAX_CLIENT_WIDTH, MIN_CLIENT_HEIGHT, MIN_CLIENT_WIDTH};

const WINDOW_STYLE_FLAGS: u32 = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_THICKFRAME;
/// Message interne : l'état de lecture a changé (envoyé depuis les threads audio).
pub const WM_STATE_CHANGED: u32 = WM_APP + 1;

const ROOT_BG: u32 = rgb(0x0f, 0x0f, 0x14);
const PANEL_BG: u32 = rgb(0x14, 0x14, 0x20);
const BUTTON_BG: u32 = rgb(0x2b, 0x2b, 0x40);
const BUTTON_ACTIVE_BG: u32 = rgb(0x3a, 0x3a, 0x55);
const PLAY_BG: u32 = rgb(0x1f, 0x6f, 0xeb);
const PLAY_ACTIVE_BG: u32 = rgb(0x2b, 0x7c, 0xff);
const LABEL_FG: u32 = rgb(0xaa, 0xaa, 0xaa);
const WHITE: u32 = rgb(0xff, 0xff, 0xff);
const HANDLE_BORDER: u32 = rgb(0x70, 0x70, 0x95);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Load,
    Play,
    Previous,
    Next,
    Backward,
    Forward,
    Mute,
    Volume,
}

struct Fonts {
    title: windows_sys::Win32::Graphics::Gdi::HFONT,
    label: windows_sys::Win32::Graphics::Gdi::HFONT,
    ui: windows_sys::Win32::Graphics::Gdi::HFONT,
    symbol: windows_sys::Win32::Graphics::Gdi::HFONT,
}

impl Fonts {
    fn new(dpi: u32) -> Self {
        let pt = |points: i32| (points * dpi as i32 + 36) / 72;
        Fonts {
            title: create_font("Arial", pt(18), true),
            label: create_font("Arial", pt(11), false),
            ui: create_font("Segoe UI", pt(9), false),
            symbol: create_font("Segoe UI Symbol", pt(10), false),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        delete_font(self.title);
        delete_font(self.label);
        delete_font(self.ui);
        delete_font(self.symbol);
    }
}

struct Layout {
    panel: RECT,
    title: RECT,
    buttons: [(Hit, RECT); 7],
    volume: RECT,
    label: RECT,
}

fn compute_layout(client: &RECT, dpi: u32) -> Layout {
    let areas = window_layout::compute(client.right - client.left, client.bottom - client.top, dpi);
    let r = |a: window_layout::Area| rect(a[0], a[1], a[2], a[3]);
    Layout {
        panel: r(areas.panel),
        title: r(areas.title),
        buttons: [
            (Hit::Load, r(areas.load)),
            (Hit::Play, r(areas.play)),
            (Hit::Previous, r(areas.previous)),
            (Hit::Next, r(areas.next)),
            (Hit::Backward, r(areas.backward)),
            (Hit::Forward, r(areas.forward)),
            (Hit::Mute, r(areas.mute)),
        ],
        volume: r(areas.volume),
        label: r(areas.label),
    }
}

pub struct MainWindow {
    hwnd: Cell<HWND>,
    core: Arc<PlayerState>,
    dpi: Cell<u32>,
    fonts: RefCell<Option<Fonts>>,
    hover: Cell<Option<Hit>>,
    pressed: Cell<Option<Hit>>,
    tracking_leave: Cell<bool>,
    explorer_open: Cell<bool>,
}

impl MainWindow {
    fn client_rect(&self) -> RECT {
        let mut r: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(self.hwnd.get(), &mut r) };
        r
    }

    fn layout(&self) -> Layout {
        compute_layout(&self.client_rect(), self.dpi.get())
    }

    fn ensure_fonts(&self) {
        if self.fonts.borrow().is_none() {
            *self.fonts.borrow_mut() = Some(Fonts::new(self.dpi.get()));
        }
    }

    fn invalidate(&self, r: Option<&RECT>) {
        unsafe {
            InvalidateRect(self.hwnd.get(), r.map_or(null(), |r| r as *const RECT), 0);
        }
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<Hit> {
        let layout = self.layout();
        for (hit, r) in layout.buttons.iter() {
            if contains(r, x, y) {
                return Some(*hit);
            }
        }
        if contains(&layout.volume, x, y) {
            return Some(Hit::Volume);
        }
        None
    }

    fn rect_of(&self, hit: Hit) -> RECT {
        let layout = self.layout();
        if hit == Hit::Volume {
            return layout.volume;
        }
        layout.buttons.iter().find(|(h, _)| *h == hit).map(|(_, r)| *r).unwrap_or(layout.panel)
    }

    // ---- dessin ---------------------------------------------------------------

    fn draw(&self, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
        self.ensure_fonts();
        let fonts_ref = self.fonts.borrow();
        let fonts = fonts_ref.as_ref().expect("polices");
        let client = self.client_rect();
        let layout = compute_layout(&client, self.dpi.get());
        let dpi = self.dpi.get();
        let s = |v: i32| scale(v, dpi);

        fill_rect(hdc, &client, ROOT_BG);
        fill_rect(hdc, &layout.panel, PANEL_BG);
        draw_text(hdc, fonts.title, WHITE, "SONORA", &layout.title, true, false);

        let playing = self.core.engine.is_playing();
        for (hit, r) in layout.buttons.iter() {
            let active = self.hover.get() == Some(*hit) || self.pressed.get() == Some(*hit);
            let (bg, active_bg) = if *hit == Hit::Play { (PLAY_BG, PLAY_ACTIVE_BG) } else { (BUTTON_BG, BUTTON_ACTIVE_BG) };
            fill_rect(hdc, r, if active { active_bg } else { bg });
            match hit {
                Hit::Load => draw_text(hdc, fonts.ui, WHITE, "Choisir musique / dossier", r, true, false),
                Hit::Play => {
                    let (sym, txt) = if playing { ("⏸", "PAUSE") } else { ("▶", "PLAY") };
                    draw_symbol_and_text(hdc, fonts.symbol, fonts.ui, WHITE, sym, txt, r, s(6));
                }
                Hit::Previous => draw_symbol_and_text(hdc, fonts.symbol, fonts.ui, WHITE, "⏮", "", r, 0),
                Hit::Next => draw_symbol_and_text(hdc, fonts.symbol, fonts.ui, WHITE, "⏭", "", r, 0),
                Hit::Backward => draw_text(hdc, fonts.ui, WHITE, "-10s", r, true, false),
                Hit::Forward => draw_text(hdc, fonts.ui, WHITE, "+10s", r, true, false),
                Hit::Mute => draw_text(hdc, fonts.ui, WHITE, volume_control::mute_label(&self.core), r, true, false),
                Hit::Volume => {}
            }
        }

        // curseur de volume : libellé, valeur, rail et poignée
        let v = layout.volume;
        let volume = self.core.lock().user_volume;
        draw_text(hdc, fonts.ui, WHITE, "VOLUME", &rect(v.left, v.top, v.left + s(120), v.top + s(18)), false, false);
        let trough = rect_xywh(v.left, v.top + s(30), v.right - v.left, s(12));
        fill_rect(hdc, &trough, BUTTON_BG);
        let handle_w = s(16);
        let usable = ((v.right - v.left) - handle_w).max(1);
        let cx = v.left + handle_w / 2 + (volume.clamp(0, 100) * usable) / 100;
        let handle = rect_xywh(cx - handle_w / 2, trough.top + s(6) - s(12), handle_w, s(24));
        fill_rect(hdc, &handle, HANDLE_BORDER);
        fill_rect(hdc, &rect(handle.left + 1, handle.top + 1, handle.right - 1, handle.bottom - 1), BUTTON_ACTIVE_BG);
        let value_rect = rect(cx - s(20), v.top + s(8), cx + s(20), v.top + s(26));
        draw_text(hdc, fonts.ui, WHITE, &volume.to_string(), &value_rect, true, false);

        // libellé du morceau
        let label = self.core.lock().label.clone();
        draw_text(hdc, fonts.label, LABEL_FG, &label, &layout.label, true, true);
    }

    fn paint(&self) {
        let mut ps: PAINTSTRUCT = unsafe { zeroed() };
        unsafe {
            let hdc = BeginPaint(self.hwnd.get(), &mut ps);
            let rc = ps.rcPaint;
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            if w > 0 && h > 0 {
                // Double tampon limité à la zone à redessiner (mémoire temporaire minimale).
                let mem = CreateCompatibleDC(hdc);
                let bitmap = CreateCompatibleBitmap(hdc, w, h);
                let old = SelectObject(mem, bitmap as HGDIOBJ);
                SetWindowOrgEx(mem, rc.left, rc.top, null_mut());
                self.draw(mem);
                BitBlt(hdc, rc.left, rc.top, w, h, mem, rc.left, rc.top, SRCCOPY);
                SelectObject(mem, old);
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(mem);
            }
            EndPaint(self.hwnd.get(), &ps);
        }
    }

    // ---- actions --------------------------------------------------------------

    fn set_volume_from_x(&self, x: i32) {
        let layout = self.layout();
        let v = layout.volume;
        let handle_w = scale(16, self.dpi.get());
        let usable = ((v.right - v.left) - handle_w).max(1);
        let fraction = (x - v.left - handle_w / 2) as f64 / usable as f64;
        let value = (fraction.clamp(0.0, 1.0) * 100.0).round();
        volume_control::set_volume(&self.core, value);
        self.invalidate(Some(&rect(v.left, v.top, v.right, v.bottom)));
        // le bouton MUTE peut avoir changé d'état
        self.invalidate(Some(&self.rect_of(Hit::Mute)));
    }

    fn set_playlist(&self, list: Vec<std::path::PathBuf>) {
        if list.is_empty() {
            return;
        }
        {
            let mut g = self.core.lock();
            g.playlist = list;
            g.index = 0;
        }
        playlist_navigation::play_requested_by_user(&self.core, 0);
    }

    fn load_media(&self) {
        let hwnd = self.hwnd.get();
        if win32_dialogs::ask_choose_folders(hwnd) {
            self.explorer_open.set(true);
            let core = self.core.clone();
            let owner = hwnd;
            browser_window::open(
                hwnd,
                self.dpi.get(),
                Box::new(move |folders| {
                    // Appelé à la fermeture de l'explorateur (liste vide = annulation).
                    let playlist = media_selection::playlist_from_folders(&folders);
                    if !playlist.is_empty() {
                        {
                            let mut g = core.lock();
                            g.playlist = playlist;
                            g.index = 0;
                        }
                        playlist_navigation::play_requested_by_user(&core, 0);
                    }
                    unsafe {
                        PostMessageW(owner, WM_STATE_CHANGED, 1, 0);
                    }
                }),
            );
        } else {
            let files = win32_dialogs::pick_files(hwnd);
            self.set_playlist(files);
        }
    }

    fn activate(&self, hit: Hit) {
        match hit {
            Hit::Load => self.load_media(),
            Hit::Play => playback_commands::execute(&self.core, PlaybackCommand::TogglePlayPause),
            Hit::Previous => playback_commands::execute(&self.core, PlaybackCommand::PreviousTrack),
            Hit::Next => playback_commands::execute(&self.core, PlaybackCommand::NextTrack),
            Hit::Backward => seek_controls::sub_10s(&self.core),
            Hit::Forward => seek_controls::add_10s(&self.core),
            Hit::Mute => playback_commands::execute(&self.core, PlaybackCommand::ToggleMute),
            Hit::Volume => {}
        }
        self.invalidate(None);
    }

    // ---- messages ---------------------------------------------------------------

    fn on_mouse_move(&self, x: i32, y: i32) {
        if !self.tracking_leave.get() {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd.get(),
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut tme) };
            self.tracking_leave.set(true);
        }
        if self.pressed.get() == Some(Hit::Volume) {
            self.set_volume_from_x(x);
            return;
        }
        let hit = self.hit_test(x, y);
        let hit = if hit == Some(Hit::Volume) { None } else { hit };
        if hit != self.hover.get() {
            let old = self.hover.replace(hit);
            if let Some(o) = old {
                self.invalidate(Some(&self.rect_of(o)));
            }
            if let Some(n) = hit {
                self.invalidate(Some(&self.rect_of(n)));
            }
        }
    }

    fn on_button_down(&self, x: i32, y: i32) {
        if self.explorer_open.get() {
            return;
        }
        if let Some(hit) = self.hit_test(x, y) {
            self.pressed.set(Some(hit));
            unsafe { SetCapture(self.hwnd.get()) };
            if hit == Hit::Volume {
                self.set_volume_from_x(x);
            } else {
                self.invalidate(Some(&self.rect_of(hit)));
            }
        }
    }

    fn on_button_up(&self, x: i32, y: i32) {
        let pressed = self.pressed.take();
        unsafe { ReleaseCapture() };
        if let Some(hit) = pressed {
            if hit != Hit::Volume && self.hit_test(x, y) == Some(hit) {
                self.activate(hit);
            } else {
                self.invalidate(Some(&self.rect_of(hit)));
            }
        }
    }

    fn on_dpi_changed(&self, new_dpi: u32, suggested: *const RECT) {
        self.dpi.set(new_dpi);
        *self.fonts.borrow_mut() = None;
        set_window_icons(self.hwnd.get(), new_dpi);
        if !suggested.is_null() {
            let r = unsafe { *suggested };
            unsafe {
                SetWindowPos(self.hwnd.get(), null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        self.invalidate(None);
    }
}

/// Taille extérieure de la fenêtre (cadre et barre de titre compris) pour une zone cliente de
/// `client_w` x `client_h` pixels à 96 DPI, au DPI donné.
fn frame_size_for_client(client_w: i32, client_h: i32, dpi: u32) -> (i32, i32) {
    let mut r = rect_xywh(0, 0, scale(client_w, dpi), scale(client_h, dpi));
    unsafe { AdjustWindowRectExForDpi(&mut r, WINDOW_STYLE_FLAGS, 0, 0, dpi) };
    (r.right - r.left, r.bottom - r.top)
}

fn lparam_x(l: LPARAM) -> i32 {
    (l & 0xFFFF) as i16 as i32
}

fn lparam_y(l: LPARAM) -> i32 {
    ((l >> 16) & 0xFFFF) as i16 as i32
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        let this = &*(cs.lpCreateParams as *const MainWindow);
        this.hwnd.set(hwnd);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const MainWindow;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let this = &*ptr;

    match msg {
        WM_PAINT => {
            this.paint();
            0
        }
        WM_ERASEBKGND => 1,
        WM_GETMINMAXINFO => {
            // Taille d'origine = taille maximale ; minimum 480 x 320 (zone cliente, 96 DPI).
            let dpi = this.dpi.get();
            let info = &mut *(lparam as *mut MINMAXINFO);
            let (min_w, min_h) = frame_size_for_client(MIN_CLIENT_WIDTH, MIN_CLIENT_HEIGHT, dpi);
            let (max_w, max_h) = frame_size_for_client(MAX_CLIENT_WIDTH, MAX_CLIENT_HEIGHT, dpi);
            info.ptMinTrackSize.x = min_w;
            info.ptMinTrackSize.y = min_h;
            info.ptMaxTrackSize.x = max_w;
            info.ptMaxTrackSize.y = max_h;
            0
        }
        WM_SIZE => {
            if wparam as u32 == SIZE_MINIMIZED {
                memory_trimming::trim_working_set();
            } else {
                this.invalidate(None);
            }
            0
        }
        WM_HOTKEY => {
            if let Some(command) = media_keys::command_for_hotkey(wparam as i32) {
                playback_commands::execute(&this.core, command);
                this.invalidate(None);
            }
            0
        }
        WM_APPCOMMAND => match media_keys::command_for_app_command(media_keys::app_command_from_lparam(lparam)) {
            Some(command) => {
                playback_commands::execute(&this.core, command);
                this.invalidate(None);
                1 // traitée : Windows n'applique pas son action par défaut (ex. volume système)
            }
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        },
        WM_MOUSEMOVE => {
            this.on_mouse_move(lparam_x(lparam), lparam_y(lparam));
            0
        }
        WM_MOUSELEAVE => {
            this.tracking_leave.set(false);
            if let Some(old) = this.hover.take() {
                this.invalidate(Some(&this.rect_of(old)));
            }
            0
        }
        WM_LBUTTONDOWN => {
            this.on_button_down(lparam_x(lparam), lparam_y(lparam));
            0
        }
        WM_LBUTTONUP => {
            this.on_button_up(lparam_x(lparam), lparam_y(lparam));
            0
        }
        WM_STATE_CHANGED => {
            if wparam == 1 {
                this.explorer_open.set(false);
            }
            this.invalidate(None);
            0
        }
        WM_DPICHANGED => {
            this.on_dpi_changed((wparam & 0xFFFF) as u32, lparam as *const RECT);
            0
        }
        WM_DESTROY => {
            media_keys::unregister(hwnd);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            // Libère l'état de la fenêtre (créé par Box::into_raw dans `create`).
            drop(Box::from_raw(ptr as *mut MainWindow));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Crée et affiche la fenêtre principale ; renvoie son HWND.
pub fn create(core: Arc<PlayerState>) -> HWND {
    unsafe {
        let hinstance = GetModuleHandleW(null());
        let class_name = wide(CLASS_NAME);

        let system_dpi = GetDpiForSystem().max(96);
        let icon_big = big_icon(system_dpi);
        let icon_small = small_icon(system_dpi);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: icon_big,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: icon_small,
        };
        RegisterClassExW(&wc);

        let dpi = system_dpi;
        let style = WINDOW_STYLE_FLAGS; // redimensionnable entre le minimum et la taille d'origine
        let mut frame = rect_xywh(0, 0, scale(MAX_CLIENT_WIDTH, dpi), scale(MAX_CLIENT_HEIGHT, dpi));
        AdjustWindowRectExForDpi(&mut frame, style, 0, 0, dpi);
        let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;

        let state = Box::new(MainWindow {
            hwnd: Cell::new(null_mut()),
            core,
            dpi: Cell::new(dpi),
            fonts: RefCell::new(None),
            hover: Cell::new(None),
            pressed: Cell::new(None),
            tracking_leave: Cell::new(false),
            explorer_open: Cell::new(false),
        });
        let raw = Box::into_raw(state);

        let title = wide(WINDOW_TITLE);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            x.max(0),
            y.max(0),
            w,
            h,
            null_mut(),
            null_mut(),
            hinstance,
            raw as *const std::ffi::c_void,
        );
        if hwnd.is_null() {
            drop(Box::from_raw(raw));
            return null_mut();
        }

        // DPI réel du moniteur où la fenêtre est apparue.
        let real_dpi = GetDpiForWindow(hwnd);
        if real_dpi != 0 && real_dpi != dpi {
            (*raw).dpi.set(real_dpi);
            let (fw, fh) = frame_size_for_client(MAX_CLIENT_WIDTH, MAX_CLIENT_HEIGHT, real_dpi);
            SetWindowPos(hwnd, null_mut(), 0, 0, fw, fh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE);
        }

        use_dark_title_bar(hwnd);
        set_window_icons(hwnd, (*raw).dpi.get());
        ShowWindow(hwnd, SW_SHOW);
        // Touches multimédia (lecture/pause, suivant, précédent, arrêt, volume, muet).
        media_keys::register(hwnd);
        hwnd
    }
}
