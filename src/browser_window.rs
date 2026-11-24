// -- browser_window.rs -- //   (fenêtre « File Explorer » de ld_ds.pyw)
//
// Fenêtre native de 900x520 : barre de chemin (contrôle EDIT), liste de dossiers
// dessinée en GDI avec sélection par glissement, bouton « Valider sélection ».
// La logique (navigation, sélection, filtres de dossiers système) est dans folder_browser.rs.

use std::cell::{Cell, RefCell};
use std::mem::zeroed;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject,
    EndPaint, InvalidateRect, SelectObject, SetBkColor, SetTextColor, SetWindowOrgEx, HBRUSH, HDC, HFONT,
    HGDIOBJ, PAINTSTRUCT, SRCCOPY,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetScrollInfo, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextW, LoadCursorW, PostMessageW, RegisterClassExW,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    CREATESTRUCTW, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, GWLP_WNDPROC, ES_AUTOHSCROLL, IDC_ARROW,
    SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBTRACK, SB_VERT, SCROLLINFO,
    SIF_ALL, SIF_TRACKPOS, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, WM_APP, WM_CHAR,
    WM_CLOSE, WM_CTLCOLOREDIT, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_SETFONT,
    WM_SIZE, WM_VSCROLL, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL, WS_OVERLAPPED, WS_CLIPCHILDREN,
};

use windows_sys::Win32::UI::Controls::SetScrollInfo;

use crate::folder_browser::FolderExplorer;
use crate::win32_support::*;

const WM_MOUSELEAVE: u32 = 0x02A3;
const WINDOW_CLASS: &str = "SonoraFolderBrowserWindow";
const LIST_CLASS: &str = "SonoraFolderBrowserList";
const WM_ENTER_PRESSED: u32 = WM_APP + 10;

const BACKGROUND: u32 = rgb(0x1e, 0x1e, 0x1e);
const BUTTON: u32 = rgb(0x2d, 0x2d, 0x30);
const BUTTON_HOVER: u32 = rgb(0x3e, 0x3e, 0x42);
const ENTRY_BACKGROUND: u32 = rgb(0x25, 0x25, 0x26);
const SELECTED_ROW: u32 = rgb(0x09, 0x47, 0x71);
const VALIDATE: u32 = rgb(0x00, 0x78, 0xd7);
const VALIDATE_HOVER: u32 = rgb(0x1a, 0x8a, 0xe6);
const WHITE: u32 = rgb(0xff, 0xff, 0xff);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Back,
    Go,
    Refresh,
    Validate,
}

struct Fonts {
    ui: HFONT,
    rows: HFONT,
    symbol: HFONT,
    emoji: HFONT,
}

impl Fonts {
    fn new(dpi: u32) -> Self {
        let pt = |points: i32| (points * dpi as i32 + 36) / 72;
        Fonts {
            ui: create_font("Segoe UI", pt(9), false),
            rows: create_font("Segoe UI", pt(10), false),
            symbol: create_font("Segoe UI Symbol", pt(10), false),
            emoji: create_font("Segoe UI Emoji", pt(10), false),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        delete_font(self.ui);
        delete_font(self.rows);
        delete_font(self.symbol);
        delete_font(self.emoji);
    }
}

struct Layout {
    back: RECT,
    entry: RECT,
    go: RECT,
    refresh: RECT,
    list: RECT,
    validate: RECT,
}

fn layout(dpi: u32) -> Layout {
    let s = |v: i32| scale(v, dpi);
    Layout {
        back: rect_xywh(s(8), s(6), s(30), s(26)),
        entry: rect_xywh(s(44), s(6), s(772), s(26)),
        go: rect_xywh(s(822), s(6), s(34), s(26)),
        refresh: rect_xywh(s(862), s(6), s(30), s(26)),
        list: rect_xywh(s(8), s(38), s(884), s(436)),
        validate: rect_xywh(s(762), s(484), s(130), s(28)),
    }
}

struct BrowserWindow {
    hwnd: Cell<HWND>,
    owner: HWND,
    list: Cell<HWND>,
    edit: Cell<HWND>,
    dpi: Cell<u32>,
    explorer: RefCell<FolderExplorer>,
    fonts: RefCell<Option<Fonts>>,
    hover: Cell<Option<Button>>,
    pressed: Cell<Option<Button>>,
    tracking_leave: Cell<bool>,
    scroll_y: Cell<i32>,
    entry_brush: Cell<HBRUSH>,
    on_finish: RefCell<Option<Box<dyn FnOnce(Vec<PathBuf>)>>>,
}

static ORIGINAL_EDIT_PROC: AtomicIsize = AtomicIsize::new(0);

fn lparam_x(l: LPARAM) -> i32 {
    (l & 0xFFFF) as i16 as i32
}

fn lparam_y(l: LPARAM) -> i32 {
    ((l >> 16) & 0xFFFF) as i16 as i32
}

impl BrowserWindow {
    fn ensure_fonts(&self) {
        if self.fonts.borrow().is_none() {
            *self.fonts.borrow_mut() = Some(Fonts::new(self.dpi.get()));
        }
    }

    fn row_height(&self) -> i32 {
        scale(28, self.dpi.get())
    }

    fn list_client_height(&self) -> i32 {
        let mut r: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(self.list.get(), &mut r) };
        r.bottom - r.top
    }

    fn max_scroll(&self) -> i32 {
        let content = self.explorer.borrow().rows.len() as i32 * self.row_height();
        (content - self.list_client_height()).max(0)
    }

    fn update_scrollbar(&self) {
        let content = self.explorer.borrow().rows.len() as i32 * self.row_height();
        let page = self.list_client_height();
        self.scroll_y.set(self.scroll_y.get().clamp(0, self.max_scroll()));
        let info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            nMin: 0,
            nMax: (content - 1).max(0),
            nPage: page.max(0) as u32,
            nPos: self.scroll_y.get(),
            nTrackPos: 0,
        };
        unsafe { SetScrollInfo(self.list.get(), SB_VERT, &info, 1) };
    }

    fn scroll_to(&self, y: i32) {
        self.scroll_y.set(y.clamp(0, self.max_scroll()));
        self.update_scrollbar();
        unsafe { InvalidateRect(self.list.get(), null(), 0) };
    }

    /// Ligne sous la coordonnée `y` du contrôle liste (`None` si hors des lignes).
    fn row_at(&self, y: i32) -> Option<usize> {
        let index = (y + self.scroll_y.get()).div_euclid(self.row_height());
        let count = self.explorer.borrow().rows.len() as i32;
        if y >= 0 && index >= 0 && index < count { Some(index as usize) } else { None }
    }

    /// Ligne la plus proche, bornée (glissement au-delà des bords de la liste).
    fn row_clamped(&self, y: i32) -> Option<usize> {
        let count = self.explorer.borrow().rows.len() as i32;
        if count == 0 {
            return None;
        }
        let index = (y + self.scroll_y.get()).div_euclid(self.row_height());
        Some(index.clamp(0, count - 1) as usize)
    }

    fn sync_after_navigation(&self) {
        let text = wide(&self.explorer.borrow().current_path);
        unsafe { SetWindowTextW(self.edit.get(), text.as_ptr()) };
        self.scroll_y.set(0);
        self.update_scrollbar();
        unsafe { InvalidateRect(self.list.get(), null(), 0) };
    }

    // ---- dessin de la fenêtre parente -------------------------------------------

    fn draw_parent(&self, hdc: HDC) {
        self.ensure_fonts();
        let fonts_ref = self.fonts.borrow();
        let fonts = fonts_ref.as_ref().expect("polices");
        let dpi = self.dpi.get();
        let l = layout(dpi);
        let mut client: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(self.hwnd.get(), &mut client) };
        fill_rect(hdc, &client, BACKGROUND);
        fill_rect(hdc, &l.entry, ENTRY_BACKGROUND);

        for (which, r) in [
            (Button::Back, l.back),
            (Button::Go, l.go),
            (Button::Refresh, l.refresh),
            (Button::Validate, l.validate),
        ] {
            let active = self.hover.get() == Some(which) || self.pressed.get() == Some(which);
            let (bg, hover_bg) = if which == Button::Validate { (VALIDATE, VALIDATE_HOVER) } else { (BUTTON, BUTTON_HOVER) };
            fill_rect(hdc, &r, if active { hover_bg } else { bg });
            match which {
                Button::Back => draw_symbol_and_text(hdc, fonts.symbol, fonts.ui, WHITE, "⬅", "", &r, 0),
                Button::Refresh => draw_symbol_and_text(hdc, fonts.symbol, fonts.ui, WHITE, "⟳", "", &r, 0),
                Button::Go => draw_text(hdc, fonts.ui, WHITE, "Go", &r, true, false),
                Button::Validate => draw_text(hdc, fonts.ui, WHITE, "Valider sélection", &r, true, false),
            }
        }
    }

    fn button_rect(&self, which: Button) -> RECT {
        let l = layout(self.dpi.get());
        match which {
            Button::Back => l.back,
            Button::Go => l.go,
            Button::Refresh => l.refresh,
            Button::Validate => l.validate,
        }
    }

    fn button_at(&self, x: i32, y: i32) -> Option<Button> {
        [Button::Back, Button::Go, Button::Refresh, Button::Validate]
            .into_iter()
            .find(|b| contains(&self.button_rect(*b), x, y))
    }

    // ---- dessin de la liste ------------------------------------------------------

    fn draw_list(&self, hdc: HDC, client: &RECT) {
        self.ensure_fonts();
        let fonts_ref = self.fonts.borrow();
        let fonts = fonts_ref.as_ref().expect("polices");
        fill_rect(hdc, client, ENTRY_BACKGROUND);

        let row_h = self.row_height();
        let explorer = self.explorer.borrow();
        let first = (self.scroll_y.get() / row_h).max(0) as usize;
        let pad = scale(8, self.dpi.get());
        let glyph_w = scale(24, self.dpi.get());
        for (i, row) in explorer.rows.iter().enumerate().skip(first) {
            let top = i as i32 * row_h - self.scroll_y.get();
            if top >= client.bottom {
                break;
            }
            let r = rect(client.left, top, client.right, top + row_h);
            if explorer.selected.contains(&i) {
                fill_rect(hdc, &r, SELECTED_ROW);
            }
            // « symbole nom » : le symbole utilise la police adaptée (flèche / dossier).
            let (glyph, name) = row.text.split_once(' ').unwrap_or(("", row.text.as_str()));
            let glyph_font = if row.is_parent { fonts.symbol } else { fonts.emoji };
            draw_text(hdc, glyph_font, WHITE, glyph, &rect(r.left + pad, r.top, r.left + pad + glyph_w, r.bottom), false, false);
            draw_text(hdc, fonts.rows, WHITE, name, &rect(r.left + pad + glyph_w, r.top, r.right, r.bottom), false, true);
        }
    }

    fn paint_list(&self, hwnd: HWND) {
        let mut ps: PAINTSTRUCT = unsafe { zeroed() };
        unsafe {
            let hdc = BeginPaint(hwnd, &mut ps);
            let rc = ps.rcPaint;
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            if w > 0 && h > 0 {
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                let mem = CreateCompatibleDC(hdc);
                let bitmap = CreateCompatibleBitmap(hdc, w, h);
                let old = SelectObject(mem, bitmap as HGDIOBJ);
                SetWindowOrgEx(mem, rc.left, rc.top, null_mut());
                self.draw_list(mem, &client);
                BitBlt(hdc, rc.left, rc.top, w, h, mem, rc.left, rc.top, SRCCOPY);
                SelectObject(mem, old);
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(mem);
            }
            EndPaint(hwnd, &ps);
        }
    }

    fn paint_parent(&self) {
        let mut ps: PAINTSTRUCT = unsafe { zeroed() };
        unsafe {
            let hdc = BeginPaint(self.hwnd.get(), &mut ps);
            let rc = ps.rcPaint;
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            if w > 0 && h > 0 {
                let mem = CreateCompatibleDC(hdc);
                let bitmap = CreateCompatibleBitmap(hdc, w, h);
                let old = SelectObject(mem, bitmap as HGDIOBJ);
                SetWindowOrgEx(mem, rc.left, rc.top, null_mut());
                self.draw_parent(mem);
                BitBlt(hdc, rc.left, rc.top, w, h, mem, rc.left, rc.top, SRCCOPY);
                SelectObject(mem, old);
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(mem);
            }
            EndPaint(self.hwnd.get(), &ps);
        }
    }

    // ---- fermeture --------------------------------------------------------------

    fn finish(&self, result: Vec<PathBuf>) {
        let callback = self.on_finish.borrow_mut().take();
        unsafe {
            EnableWindow(self.owner, 1);
            SetForegroundWindow(self.owner);
        }
        if let Some(cb) = callback {
            cb(result);
        }
        unsafe { DestroyWindow(self.hwnd.get()) };
    }

    fn activate(&self, which: Button) {
        match which {
            Button::Back => {
                self.explorer.borrow_mut().go_back();
                self.sync_after_navigation();
            }
            Button::Go => {
                let mut buffer = [0u16; 2048];
                let n = unsafe { GetWindowTextW(self.edit.get(), buffer.as_mut_ptr(), buffer.len() as i32) };
                if n >= 0 {
                    self.explorer.borrow_mut().current_path = String::from_utf16_lossy(&buffer[..n as usize]);
                }
                self.explorer.borrow_mut().open_path();
                self.sync_after_navigation();
            }
            Button::Refresh => {
                self.explorer.borrow_mut().refresh();
                self.sync_after_navigation();
            }
            Button::Validate => {
                self.explorer.borrow_mut().validate_selection();
                let result = self.explorer.borrow_mut().result.take().unwrap_or_default();
                self.finish(result);
            }
        }
    }

    fn layout_children(&self) {
        let l = layout(self.dpi.get());
        let s = |v: i32| scale(v, self.dpi.get());
        unsafe {
            // Le champ EDIT occupe l'intérieur du cadre « entry », centré verticalement.
            let h = s(18);
            SetWindowPos(
                self.edit.get(),
                null_mut(),
                l.entry.left + s(6),
                l.entry.top + ((l.entry.bottom - l.entry.top) - h) / 2,
                (l.entry.right - l.entry.left) - s(12),
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            SetWindowPos(
                self.list.get(),
                null_mut(),
                l.list.left,
                l.list.top,
                l.list.right - l.list.left,
                l.list.bottom - l.list.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.update_scrollbar();
    }

    fn apply_fonts_to_edit(&self) {
        self.ensure_fonts();
        if let Some(f) = self.fonts.borrow().as_ref() {
            unsafe { SendMessageW(self.edit.get(), WM_SETFONT, f.ui as usize, 1) };
        }
    }
}

// ---- procédures de fenêtre ---------------------------------------------------------

unsafe extern "system" fn edit_subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CHAR && wparam == 13 {
        // Entrée dans le champ de chemin = « Go ».
        let parent = windows_sys::Win32::UI::WindowsAndMessaging::GetParent(hwnd);
        PostMessageW(parent, WM_ENTER_PRESSED, 0, 0);
        return 0;
    }
    let original: isize = ORIGINAL_EDIT_PROC.load(Ordering::Acquire);
    let proc: windows_sys::Win32::UI::WindowsAndMessaging::WNDPROC = std::mem::transmute(original);
    CallWindowProcW(proc, hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn parent_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        let this = &*(cs.lpCreateParams as *const BrowserWindow);
        this.hwnd.set(hwnd);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const BrowserWindow;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let this = &*ptr;

    match msg {
        WM_PAINT => {
            this.paint_parent();
            0
        }
        WM_ERASEBKGND => 1,
        WM_CTLCOLOREDIT => {
            let hdc = wparam as HDC;
            SetTextColor(hdc, WHITE);
            SetBkColor(hdc, ENTRY_BACKGROUND);
            this.entry_brush.get() as LRESULT
        }
        WM_ENTER_PRESSED => {
            this.activate(Button::Go);
            0
        }
        WM_MOUSEMOVE => {
            if !this.tracking_leave.get() {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
                this.tracking_leave.set(true);
            }
            let hit = this.button_at(lparam_x(lparam), lparam_y(lparam));
            if hit != this.hover.get() {
                let old = this.hover.replace(hit);
                for b in [old, hit].into_iter().flatten() {
                    InvalidateRect(hwnd, &this.button_rect(b), 0);
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            this.tracking_leave.set(false);
            if let Some(old) = this.hover.take() {
                InvalidateRect(hwnd, &this.button_rect(old), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            if let Some(b) = this.button_at(lparam_x(lparam), lparam_y(lparam)) {
                this.pressed.set(Some(b));
                SetCapture(hwnd);
                InvalidateRect(hwnd, &this.button_rect(b), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            let pressed = this.pressed.take();
            ReleaseCapture();
            if let Some(b) = pressed {
                if this.button_at(lparam_x(lparam), lparam_y(lparam)) == Some(b) {
                    this.activate(b);
                } else {
                    InvalidateRect(hwnd, &this.button_rect(b), 0);
                }
            }
            0
        }
        WM_DPICHANGED => {
            this.dpi.set((wparam & 0xFFFF) as u32);
            *this.fonts.borrow_mut() = None;
            let r = &*(lparam as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            this.apply_fonts_to_edit();
            this.layout_children();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CLOSE => {
            // Fermeture par la croix : annulation (rien n'est validé).
            this.finish(Vec::new());
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let boxed = Box::from_raw(ptr as *mut BrowserWindow);
            DeleteObject(boxed.entry_brush.get() as HGDIOBJ);
            drop(boxed);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn list_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const BrowserWindow;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let this = &*ptr;

    match msg {
        WM_PAINT => {
            this.paint_list(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_SIZE => {
            this.update_scrollbar();
            0
        }
        WM_LBUTTONDOWN => {
            // La ligne est déterminée avant d'emprunter l'explorateur en écriture.
            let row = this.row_at(lparam_y(lparam));
            this.explorer.borrow_mut().press(row);
            SetCapture(hwnd);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEMOVE => {
            if this.explorer.borrow().dragging {
                let y = lparam_y(lparam);
                // Glissement au-delà des bords : la liste défile.
                let page = this.list_client_height();
                if y < 0 {
                    this.scroll_to(this.scroll_y.get() + y / 2 - 4);
                } else if y > page {
                    this.scroll_to(this.scroll_y.get() + (y - page) / 2 + 4);
                }
                let row = this.row_clamped(y);
                this.explorer.borrow_mut().drag(row);
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            this.explorer.borrow_mut().release();
            ReleaseCapture();
            0
        }
        WM_LBUTTONDBLCLK => {
            if let Some(row) = this.row_at(lparam_y(lparam)) {
                this.explorer.borrow_mut().open_row(row);
                this.sync_after_navigation();
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16 as i32;
            this.scroll_to(this.scroll_y.get() - delta / 120 * this.row_height() * 3);
            0
        }
        WM_VSCROLL => {
            let code = (wparam & 0xFFFF) as i32;
            let row = this.row_height();
            let page = this.list_client_height();
            let current = this.scroll_y.get();
            let target = match code {
                x if x == SB_LINEUP as i32 => current - row,
                x if x == SB_LINEDOWN as i32 => current + row,
                x if x == SB_PAGEUP as i32 => current - page,
                x if x == SB_PAGEDOWN as i32 => current + page,
                x if x == SB_THUMBTRACK as i32 => {
                    let mut info = SCROLLINFO {
                        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_TRACKPOS,
                        nMin: 0,
                        nMax: 0,
                        nPage: 0,
                        nPos: 0,
                        nTrackPos: 0,
                    };
                    GetScrollInfo(hwnd, SB_VERT, &mut info);
                    info.nTrackPos
                }
                _ => current,
            };
            this.scroll_to(target);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_classes() {
    unsafe {
        let hinstance = GetModuleHandleW(null());
        let cursor = LoadCursorW(null_mut(), IDC_ARROW);
        let system_dpi = GetDpiForSystem().max(96);
        let icon = big_icon(system_dpi);
        let icon_small = small_icon(system_dpi);

        let parent_class = wide(WINDOW_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(parent_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: icon,
            hCursor: cursor,
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: parent_class.as_ptr(),
            hIconSm: icon_small,
        };
        RegisterClassExW(&wc); // déjà enregistrée à la 2e ouverture : l'échec est sans conséquence

        let list_class = wide(LIST_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(list_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: null_mut(),
            hCursor: cursor,
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: list_class.as_ptr(),
            hIconSm: null_mut(),
        };
        RegisterClassExW(&wc);
    }
}

/// Ouvre l'explorateur (modal par rapport à `owner`). `on_finish` reçoit les dossiers
/// validés, ou une liste vide si l'utilisateur a annulé / fermé la fenêtre.
pub fn open(owner: HWND, dpi_hint: u32, on_finish: Box<dyn FnOnce(Vec<PathBuf>)>) {
    register_classes();
    unsafe {
        let hinstance = GetModuleHandleW(null());
        let dpi = dpi_hint.max(96);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;

        let mut frame = rect_xywh(0, 0, scale(900, dpi), scale(520, dpi));
        AdjustWindowRectExForDpi(&mut frame, style, 0, 0, dpi);
        let (w, h) = (frame.right - frame.left, frame.bottom - frame.top);
        let x = ((GetSystemMetrics(SM_CXSCREEN) - w) / 2).max(0);
        let y = ((GetSystemMetrics(SM_CYSCREEN) - h) / 2).max(0);

        let explorer = FolderExplorer::new();
        let initial = wide(&explorer.current_path);
        let state = Box::new(BrowserWindow {
            hwnd: Cell::new(null_mut()),
            owner,
            list: Cell::new(null_mut()),
            edit: Cell::new(null_mut()),
            dpi: Cell::new(dpi),
            explorer: RefCell::new(explorer),
            fonts: RefCell::new(None),
            hover: Cell::new(None),
            pressed: Cell::new(None),
            tracking_leave: Cell::new(false),
            scroll_y: Cell::new(0),
            entry_brush: Cell::new(CreateSolidBrush(ENTRY_BACKGROUND)),
            on_finish: RefCell::new(Some(on_finish)),
        });
        let raw = Box::into_raw(state);

        let class = wide(WINDOW_CLASS);
        let title = wide("File Explorer");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            owner,
            null_mut(),
            hinstance,
            raw as *const std::ffi::c_void,
        );
        if hwnd.is_null() {
            let b = Box::from_raw(raw);
            DeleteObject(b.entry_brush.get() as HGDIOBJ);
            return;
        }
        let this = &*raw;

        let real_dpi = GetDpiForWindow(hwnd);
        if real_dpi != 0 && real_dpi != dpi {
            this.dpi.set(real_dpi);
            let mut f = rect_xywh(0, 0, scale(900, real_dpi), scale(520, real_dpi));
            AdjustWindowRectExForDpi(&mut f, style, 0, 0, real_dpi);
            SetWindowPos(hwnd, null_mut(), 0, 0, f.right - f.left, f.bottom - f.top, SWP_NOZORDER | SWP_NOACTIVATE | 0x0002);
        }

        // contrôle EDIT (chemin) et liste personnalisée
        let edit_class = wide("EDIT");
        let edit = CreateWindowExW(
            0,
            edit_class.as_ptr(),
            initial.as_ptr(),
            WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
            0,
            0,
            10,
            10,
            hwnd,
            null_mut(),
            hinstance,
            null(),
        );
        this.edit.set(edit);
        let original = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_subclass_proc as *const () as isize);
        ORIGINAL_EDIT_PROC.store(original, Ordering::Release);

        let list_class = wide(LIST_CLASS);
        let list = CreateWindowExW(
            0,
            list_class.as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL,
            0,
            0,
            10,
            10,
            hwnd,
            null_mut(),
            hinstance,
            raw as *const std::ffi::c_void,
        );
        this.list.set(list);
        use_dark_scrollbars(list);

        this.apply_fonts_to_edit();
        this.layout_children();

        use_dark_title_bar(hwnd);
        set_window_icons(hwnd, this.dpi.get());

        EnableWindow(owner, 0); // modal : la fenêtre principale est inactive pendant le choix
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(hwnd);
    }
}
