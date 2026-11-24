// -- window_layout.rs -- //
//
// Disposition RESPONSIVE de la fenêtre principale (calcul pur, sans Windows).
//
// La taille d'origine (zone cliente 900 x 520 à 96 DPI) est la taille maximale :
// la fenêtre peut être réduite jusqu'à 480 x 320. Quand elle rétrécit, les marges
// du panneau diminuent (40 -> 8 px) puis les espacements verticaux entre les
// contrôles se resserrent ; les contrôles gardent leur taille. À la taille
// maximale, le résultat est identique à la disposition d'origine.

/// Rectangle (gauche, haut, droite, bas) en pixels.
pub type Area = [i32; 4];

pub const MAX_CLIENT_WIDTH: i32 = 900;
pub const MAX_CLIENT_HEIGHT: i32 = 520;
pub const MIN_CLIENT_WIDTH: i32 = 480;
pub const MIN_CLIENT_HEIGHT: i32 = 320;

/// Somme des hauteurs des contrôles (96 DPI) et des espacements verticaux d'origine.
const CONTENT_HEIGHT: i32 = 31 + 40 + 40 + 26 + 26 + 52 + 22;
const PADDING_TOTAL: i32 = 150;
/// Marge basse réservée sous le libellé du morceau (96 DPI).
const BOTTOM_RESERVE: i32 = 10;
/// Les espacements ne descendent jamais sous 30 % de leur valeur d'origine (en millièmes).
const MIN_PADDING_PERMILLE: i32 = 300;

/// Mise à l'échelle d'une valeur de 96 DPI vers le DPI courant (arrondie).
pub fn scale(value: i32, dpi: u32) -> i32 {
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutAreas {
    pub panel: Area,
    pub title: Area,
    pub load: Area,
    pub play: Area,
    pub previous: Area,
    pub next: Area,
    pub backward: Area,
    pub forward: Area,
    pub mute: Area,
    pub volume: Area,
    pub label: Area,
}

fn xywh(x: i32, y: i32, w: i32, h: i32) -> Area {
    [x, y, x + w, y + h]
}

fn centered(cx: i32, y: i32, w: i32, h: i32) -> Area {
    xywh(cx - w / 2, y, w, h)
}

pub fn compute(client_width: i32, client_height: i32, dpi: u32) -> LayoutAreas {
    let dpi = dpi.max(96);
    let s = |v: i32| scale(v, dpi);

    // Taille logique (96 DPI) de la zone cliente -> marges du panneau, de 40 px (taille max) à 8 px (taille min).
    let logical_w = (client_width as i64 * 96 / dpi as i64) as i32;
    let logical_h = (client_height as i64 * 96 / dpi as i64) as i32;
    let margin_x = 8 + (logical_w - 520).clamp(0, 380) * 32 / 380;
    let margin_y = 8 + (logical_h - 340).clamp(0, 180) * 32 / 180;
    let (px, py) = (s(margin_x), s(margin_y));
    let (pw, ph) = (client_width - 2 * px, client_height - 2 * py);

    // Facteur d'espacement vertical (1000 = espacements d'origine).
    let panel_logical_h = (ph as i64 * 96 / dpi as i64) as i32;
    let permille =
        ((panel_logical_h - CONTENT_HEIGHT - BOTTOM_RESERVE) * 1000 / PADDING_TOTAL).clamp(MIN_PADDING_PERMILLE, 1000);
    let pad = |v: i32| s(v) * permille / 1000;

    let cx = px + pw / 2;
    let mut y = py;

    y += pad(15);
    let title = xywh(px, y, pw, s(31));
    y += s(31) + pad(15);

    y += pad(10);
    let load = centered(cx, y, s(190), s(40));
    y += s(40) + pad(10);

    y += pad(15);
    let play = centered(cx, y, s(120), s(40));
    y += s(40) + pad(15);

    y += pad(10);
    let (bw, gap) = (s(46), s(20));
    let total = 4 * bw + 3 * gap;
    let x0 = cx - total / 2;
    let control = |i: i32| xywh(x0 + i * (bw + gap), y, bw, s(26));
    let (previous, next, backward, forward) = (control(0), control(1), control(2), control(3));
    y += s(26) + pad(10);

    y += pad(10);
    let mute = centered(cx, y, s(80), s(26));
    y += s(26) + pad(10);

    y += pad(10);
    let volume = xywh(px + s(30), y, pw - 2 * s(30), s(52));
    y += s(52) + pad(10);

    y += pad(10);
    let label = xywh(px + s(20), y, pw - 2 * s(20), s(22));

    LayoutAreas {
        panel: xywh(px, py, pw, ph),
        title,
        load,
        play,
        previous,
        next,
        backward,
        forward,
        mute,
        volume,
        label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lignes de contrôles, de haut en bas (les quatre boutons de navigation partagent une ligne).
    fn rows(l: &LayoutAreas) -> Vec<(&'static str, Vec<Area>)> {
        vec![
            ("title", vec![l.title]),
            ("load", vec![l.load]),
            ("play", vec![l.play]),
            ("navigation", vec![l.previous, l.next, l.backward, l.forward]),
            ("mute", vec![l.mute]),
            ("volume", vec![l.volume]),
            ("label", vec![l.label]),
        ]
    }

    #[test]
    fn full_size_matches_the_original_layout() {
        let l = compute(900, 520, 96);
        assert_eq!(l.panel, [40, 40, 860, 480]);
        assert_eq!(l.title, [40, 55, 860, 86]);
        assert_eq!(l.load, [355, 111, 545, 151]);
        assert_eq!(l.play, [390, 176, 510, 216]);
        assert_eq!(l.previous, [328, 241, 374, 267]);
        assert_eq!(l.forward, [526, 241, 572, 267]);
        assert_eq!(l.mute, [410, 287, 490, 313]);
        assert_eq!(l.volume, [70, 333, 830, 385]);
        assert_eq!(l.label, [60, 405, 840, 427]);
    }

    #[test]
    fn every_size_between_min_and_max_keeps_controls_inside_the_panel_without_overlap() {
        for dpi in [96u32, 120, 144, 192] {
            for w in (MIN_CLIENT_WIDTH..=MAX_CLIENT_WIDTH).step_by(20) {
                for h in (MIN_CLIENT_HEIGHT..=MAX_CLIENT_HEIGHT).step_by(10) {
                    let (cw, ch) = (scale(w, dpi), scale(h, dpi));
                    let l = compute(cw, ch, dpi);
                    let mut previous_bottom = l.panel[1];
                    for (name, row) in rows(&l) {
                        let mut row_bottom = previous_bottom;
                        let mut last_right = i32::MIN;
                        for a in row {
                            assert!(a[0] >= l.panel[0] && a[2] <= l.panel[2], "{name} déborde en largeur ({w}x{h}@{dpi})");
                            assert!(a[1] >= previous_bottom, "{name} chevauche la ligne précédente ({w}x{h}@{dpi}) {a:?} < {previous_bottom}");
                            assert!(a[3] <= l.panel[3], "{name} déborde en hauteur ({w}x{h}@{dpi}) {a:?} panneau {:?}", l.panel);
                            assert!(a[0] >= last_right, "{name} : boutons qui se chevauchent ({w}x{h}@{dpi})");
                            last_right = a[2];
                            row_bottom = row_bottom.max(a[3]);
                        }
                        previous_bottom = row_bottom;
                    }
                    // le panneau reste dans la fenêtre
                    assert!(l.panel[0] >= 0 && l.panel[1] >= 0 && l.panel[2] <= cw && l.panel[3] <= ch);
                }
            }
        }
    }

    #[test]
    fn controls_stay_horizontally_centered() {
        for (w, h) in [(900, 520), (700, 400), (480, 320)] {
            let l = compute(w, h, 96);
            let center = (l.panel[0] + l.panel[2]) / 2;
            for a in [l.load, l.play, l.mute] {
                assert!(((a[0] + a[2]) / 2 - center).abs() <= 1, "{w}x{h}");
            }
        }
    }

    #[test]
    fn slider_never_gets_too_narrow() {
        let l = compute(MIN_CLIENT_WIDTH, MIN_CLIENT_HEIGHT, 96);
        assert!(l.volume[2] - l.volume[0] >= 400);
    }
}
