// -- win32_support.rs -- //
//
// Petits utilitaires Win32/GDI partagés par les deux fenêtres : chaînes larges,
// couleurs, polices, dessin de rectangles/textes, mise à l'échelle DPI, thème sombre.

use std::ffi::c_void;
use std::ptr::null;

use windows_sys::Win32::Foundation::{HWND, RECT, SIZE};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, SM_CXICON, SM_CXSMICON,
    WM_SETICON,
};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, DrawTextW, ExtTextOutW, GetTextExtentPoint32W, SelectObject, SetBkColor,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, ETO_OPAQUE, FF_DONTCARE, FW_BOLD,
    FW_NORMAL, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::SetWindowTheme;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

/// Mise à l'échelle d'une valeur de 96 DPI vers le DPI courant (arrondie).
pub fn scale(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

pub fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT { left, top, right, bottom }
}

pub fn rect_xywh(x: i32, y: i32, w: i32, h: i32) -> RECT {
    rect(x, y, x + w, y + h)
}

pub fn contains(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Police GDI : `pixels` = hauteur de cellule (positive) à l'échelle voulue.
pub fn create_font(face: &str, pixels: i32, bold: bool) -> HFONT {
    let face = wide(face);
    unsafe {
        CreateFontW(
            -pixels,
            0,
            0,
            0,
            if bold { FW_BOLD as i32 } else { FW_NORMAL as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    }
}

pub fn delete_font(font: HFONT) {
    if !font.is_null() {
        unsafe {
            DeleteObject(font as HGDIOBJ);
        }
    }
}

/// Remplit un rectangle d'une couleur unie sans créer d'objet pinceau.
pub fn fill_rect(hdc: HDC, r: &RECT, color: u32) {
    unsafe {
        SetBkColor(hdc, color);
        ExtTextOutW(hdc, 0, 0, ETO_OPAQUE, r, null(), 0, null());
    }
}

/// Dessine un texte dans `r`. `align_center` : centré horizontalement ; toujours centré verticalement.
pub fn draw_text(hdc: HDC, font: HFONT, color: u32, text: &str, r: &RECT, align_center: bool, ellipsis: bool) {
    let w = wide(text);
    let mut rr = *r;
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, color);
        let mut flags = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        if align_center {
            flags |= DT_CENTER;
        }
        if ellipsis {
            flags |= DT_END_ELLIPSIS;
        }
        DrawTextW(hdc, w.as_ptr(), (w.len() - 1) as i32, &mut rr, flags);
        SelectObject(hdc, old);
    }
}

pub fn text_width(hdc: HDC, font: HFONT, text: &str) -> i32 {
    let w = wide(text);
    let mut size = SIZE { cx: 0, cy: 0 };
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        GetTextExtentPoint32W(hdc, w.as_ptr(), (w.len() - 1) as i32, &mut size);
        SelectObject(hdc, old);
    }
    size.cx
}

/// Texte composé d'un symbole (police symboles) suivi d'un libellé (police normale), centré ensemble.
pub fn draw_symbol_and_text(
    hdc: HDC,
    symbol_font: HFONT,
    text_font: HFONT,
    color: u32,
    symbol: &str,
    label: &str,
    r: &RECT,
    gap: i32,
) {
    let sw = text_width(hdc, symbol_font, symbol);
    let tw = if label.is_empty() { 0 } else { text_width(hdc, text_font, label) };
    let total = sw + if label.is_empty() { 0 } else { gap + tw };
    let start = r.left + ((r.right - r.left) - total) / 2;
    draw_text(hdc, symbol_font, color, symbol, &rect(start, r.top, start + sw + 2, r.bottom), false, false);
    if !label.is_empty() {
        draw_text(hdc, text_font, color, label, &rect(start + sw + gap, r.top, r.right, r.bottom), false, false);
    }
}

/// Barre de titre sombre (Windows 10 1809+ / 11) ; sans effet sur les versions plus anciennes.
pub fn use_dark_title_bar(hwnd: HWND) {
    let enabled: i32 = 1;
    unsafe {
        // 20 = DWMWA_USE_IMMERSIVE_DARK_MODE (19 sur les premières versions de Windows 10)
        for attribute in [20u32, 19u32] {
            DwmSetWindowAttribute(hwnd, attribute, &enabled as *const i32 as *const c_void, 4);
        }
    }
}

/// Barres de défilement / contrôles au thème sombre.
pub fn use_dark_scrollbars(hwnd: HWND) {
    let name = wide("DarkMode_Explorer");
    unsafe {
        SetWindowTheme(hwnd, name.as_ptr(), null());
    }
}

/// Charge l'icône de l'application (ressource n° 1, dessinée par build.rs) à la taille exacte
/// demandée : Windows choisit la bonne image (16, 20, 24, 32, 40, 48, 64 ou 256 px) au lieu
/// d'étirer une image trop grande ou trop petite.
pub fn load_icon(pixels: i32) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    unsafe {
        let instance = GetModuleHandleW(null());
        LoadImageW(instance, 1 as *const u16, IMAGE_ICON, pixels, pixels, LR_DEFAULTCOLOR | LR_SHARED)
            as windows_sys::Win32::UI::WindowsAndMessaging::HICON
    }
}

/// Icône « grande » (barre des tâches, Alt+Tab) pour un DPI donné.
pub fn big_icon(dpi: u32) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    load_icon(unsafe { GetSystemMetricsForDpi(SM_CXICON, dpi) })
}

/// Icône « petite » (barre de titre) pour un DPI donné.
pub fn small_icon(dpi: u32) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
    load_icon(unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) })
}

/// Applique les deux icônes à une fenêtre, à la taille adaptée à son DPI.
pub fn set_window_icons(hwnd: HWND, dpi: u32) {
    let (big, small) = (big_icon(dpi), small_icon(dpi));
    unsafe {
        if !big.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, big as isize);
        }
        if !small.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, small as isize);
        }
    }
}
