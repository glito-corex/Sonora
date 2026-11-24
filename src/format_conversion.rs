// -- format_conversion.rs -- //
//
// Conversion du flux décodé par Symphonia vers le format réel du périphérique
// de sortie : mixage / réagencement des canaux, puis rééchantillonnage.
// 100 % Rust (rééchantillonneur sinc maison : sinc_resampler.rs), aucune dépendance native.

use crate::sinc_resampler::SincResampler;
use symphonia::core::audio::Channels;

/// Matrice de mixage `out_ch x in_ch` (ligne = canal de sortie).
pub struct ChannelMixer {
    in_ch: usize,
    out_ch: usize,
    matrix: Vec<f32>,
    identity: bool,
}

/// Contribution d'un canal source aux sorties gauche / droite d'un downmix stéréo
/// (coefficients ITU-R BS.775). Le LFE est volontairement ignoré dans le downmix.
fn stereo_weights(ch: Channels) -> (f32, f32) {
    let k = std::f32::consts::FRAC_1_SQRT_2;
    if ch == Channels::FRONT_LEFT
        || ch == Channels::FRONT_LEFT_WIDE
    {
        (1.0, 0.0)
    } else if ch == Channels::FRONT_RIGHT || ch == Channels::FRONT_RIGHT_WIDE {
        (0.0, 1.0)
    } else if ch == Channels::FRONT_CENTRE {
        (k, k)
    } else if ch == Channels::FRONT_LEFT_CENTRE {
        (k, 0.0)
    } else if ch == Channels::FRONT_RIGHT_CENTRE {
        (0.0, k)
    } else if ch == Channels::REAR_LEFT
        || ch == Channels::SIDE_LEFT
        || ch == Channels::REAR_LEFT_CENTRE
        || ch == Channels::TOP_FRONT_LEFT
        || ch == Channels::TOP_REAR_LEFT
        || ch == Channels::FRONT_LEFT_HIGH
    {
        (k, 0.0)
    } else if ch == Channels::REAR_RIGHT
        || ch == Channels::SIDE_RIGHT
        || ch == Channels::REAR_RIGHT_CENTRE
        || ch == Channels::TOP_FRONT_RIGHT
        || ch == Channels::TOP_REAR_RIGHT
        || ch == Channels::FRONT_RIGHT_HIGH
    {
        (0.0, k)
    } else if ch == Channels::REAR_CENTRE
        || ch == Channels::TOP_CENTRE
        || ch == Channels::TOP_FRONT_CENTRE
        || ch == Channels::TOP_REAR_CENTRE
        || ch == Channels::FRONT_CENTRE_HIGH
    {
        (0.5, 0.5)
    } else {
        // LFE1 / LFE2 et canaux inconnus : ignorés.
        (0.0, 0.0)
    }
}

impl ChannelMixer {
    /// `layout` : disposition des canaux telle que fournie par Symphonia
    /// (l'ordre des échantillons entrelacés suit l'ordre croissant des bits).
    pub fn new(layout: Channels, in_ch: usize, out_ch: usize) -> Self {
        let in_ch = in_ch.max(1);
        let out_ch = out_ch.max(1);

        // Même nombre de canaux : copie directe. L'ordre Symphonia
        // (FL FR FC LFE RL RR [SL SR]) est l'ordre des périphériques WASAPI.
        if in_ch == out_ch {
            let mut matrix = vec![0.0; out_ch * in_ch];
            for i in 0..in_ch {
                matrix[i * in_ch + i] = 1.0;
            }
            return Self { in_ch, out_ch, matrix, identity: true };
        }

        let mut matrix = vec![0.0f32; out_ch * in_ch];

        if in_ch == 1 {
            // Mono -> tous les canaux avant (gauche/droite) ; mono -> mono déjà traité.
            matrix[0] = 1.0;
            if out_ch >= 2 {
                matrix[in_ch] = 1.0;
            }
            return Self { in_ch, out_ch, matrix, identity: false };
        }

        // Canaux source dans l'ordre d'entrelacement.
        let mut channels: Vec<Channels> = layout.iter().collect();
        if channels.len() != in_ch {
            // Disposition incohérente : on retombe sur une hypothèse gauche/droite générique.
            channels = (0..in_ch)
                .map(|i| match i {
                    0 => Channels::FRONT_LEFT,
                    1 => Channels::FRONT_RIGHT,
                    2 => Channels::FRONT_CENTRE,
                    3 => Channels::LFE1,
                    4 => Channels::REAR_LEFT,
                    5 => Channels::REAR_RIGHT,
                    6 => Channels::SIDE_LEFT,
                    7 => Channels::SIDE_RIGHT,
                    _ => Channels::empty(),
                })
                .collect();
        }

        let mut left = vec![0.0f32; in_ch];
        let mut right = vec![0.0f32; in_ch];
        for (i, ch) in channels.iter().enumerate() {
            let (l, r) = stereo_weights(*ch);
            left[i] = l;
            right[i] = r;
        }

        // Normalisation : évite l'écrêtage quand toutes les voies sont actives.
        let sum_l: f32 = left.iter().sum();
        let sum_r: f32 = right.iter().sum();
        let norm = 1.0 / sum_l.max(sum_r).max(1.0);
        for v in left.iter_mut().chain(right.iter_mut()) {
            *v *= norm;
        }

        if out_ch == 1 {
            for i in 0..in_ch {
                matrix[i] = 0.5 * (left[i] + right[i]);
            }
        } else {
            for i in 0..in_ch {
                matrix[i] = left[i];
                matrix[in_ch + i] = right[i];
            }
            // Sorties supplémentaires (périphérique 5.1 / 7.1 avec source stéréo ou autre) : silence.
        }

        Self { in_ch, out_ch, matrix, identity: false }
    }

    /// Entrée entrelacée -> sortie planaire (`out_ch` vecteurs de `frames` échantillons).
    pub fn process(&self, input: &[f32], out: &mut [Vec<f32>]) {
        let frames = input.len() / self.in_ch;
        for ch in out.iter_mut() {
            ch.clear();
            ch.reserve(frames);
        }
        if self.identity {
            for f in 0..frames {
                let base = f * self.in_ch;
                for c in 0..self.out_ch {
                    out[c].push(input[base + c]);
                }
            }
            return;
        }
        for f in 0..frames {
            let base = f * self.in_ch;
            let frame = &input[base..base + self.in_ch];
            for c in 0..self.out_ch {
                let row = &self.matrix[c * self.in_ch..(c + 1) * self.in_ch];
                let mut acc = 0.0f32;
                for (s, w) in frame.iter().zip(row) {
                    acc += s * w;
                }
                out[c].push(acc);
            }
        }
    }
}

/// Chaîne complète : entrelacé source -> mixage -> (rééchantillonnage) -> entrelacé sortie.
pub struct Converter {
    in_rate: u32,
    in_layout: Channels,
    in_ch: usize,
    out_rate: u32,
    out_ch: usize,
    mixer: ChannelMixer,
    rate: Option<SincResampler>,
    planar: Vec<Vec<f32>>,
}

impl Converter {
    pub fn new(
        in_rate: u32,
        in_layout: Channels,
        in_ch: usize,
        out_rate: u32,
        out_ch: usize,
    ) -> Result<Self, String> {
        if in_rate == 0 || out_rate == 0 {
            return Err("fréquence d'échantillonnage invalide".into());
        }
        let rate = if in_rate != out_rate {
            Some(SincResampler::new(in_rate, out_rate, out_ch)?)
        } else {
            None
        };
        Ok(Self {
            in_rate,
            in_layout,
            in_ch,
            out_rate,
            out_ch,
            mixer: ChannelMixer::new(in_layout, in_ch, out_ch),
            rate,
            planar: vec![Vec::new(); out_ch],
        })
    }

    pub fn matches(&self, in_rate: u32, in_layout: Channels, in_ch: usize, out_rate: u32, out_ch: usize) -> bool {
        self.in_rate == in_rate
            && self.in_ch == in_ch
            && self.in_layout == in_layout
            && self.out_rate == out_rate
            && self.out_ch == out_ch
    }

    /// Ajoute à `out` les échantillons entrelacés convertis.
    pub fn process(&mut self, interleaved: &[f32], out: &mut Vec<f32>) {
        self.mixer.process(interleaved, &mut self.planar);
        match self.rate.as_mut() {
            Some(r) => r.push(&self.planar, out),
            None => {
                let frames = self.planar[0].len();
                out.reserve(frames * self.out_ch);
                for f in 0..frames {
                    for c in 0..self.out_ch {
                        out.push(self.planar[c][f]);
                    }
                }
            }
        }
    }

    /// Fin de flux : vide les tampons internes.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        if let Some(r) = self.rate.as_mut() {
            r.finish(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_stereo() {
        let m = ChannelMixer::new(Channels::FRONT_LEFT | Channels::FRONT_RIGHT, 2, 2);
        let mut out = vec![Vec::new(), Vec::new()];
        m.process(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out[0], vec![0.1, 0.3]);
        assert_eq!(out[1], vec![0.2, 0.4]);
    }

    #[test]
    fn mono_to_stereo() {
        let m = ChannelMixer::new(Channels::FRONT_LEFT, 1, 2);
        let mut out = vec![Vec::new(), Vec::new()];
        m.process(&[0.5, -0.5], &mut out);
        assert_eq!(out[0], vec![0.5, -0.5]);
        assert_eq!(out[1], vec![0.5, -0.5]);
    }

    #[test]
    fn surround_downmix_does_not_clip() {
        let layout = Channels::FRONT_LEFT
            | Channels::FRONT_RIGHT
            | Channels::FRONT_CENTRE
            | Channels::LFE1
            | Channels::REAR_LEFT
            | Channels::REAR_RIGHT;
        let m = ChannelMixer::new(layout, 6, 2);
        let mut out = vec![Vec::new(), Vec::new()];
        m.process(&[1.0; 6], &mut out);
        assert!(out[0][0] <= 1.0001 && out[0][0] > 0.9, "{}", out[0][0]);
        assert!(out[1][0] <= 1.0001 && out[1][0] > 0.9, "{}", out[1][0]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let m = ChannelMixer::new(Channels::FRONT_LEFT | Channels::FRONT_RIGHT, 2, 1);
        let mut out = vec![Vec::new()];
        m.process(&[1.0, 0.0], &mut out);
        assert!((out[0][0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn resampler_keeps_exact_length() {
        let layout = Channels::FRONT_LEFT | Channels::FRONT_RIGHT;
        for &(i, o) in &[(44100u32, 48000u32), (48000, 44100), (22050, 48000), (96000, 48000)] {
            let mut c = Converter::new(i, layout, 2, o, 2).unwrap();
            let frames_in = i as usize; // 1 seconde
            let mut data = Vec::with_capacity(frames_in * 2);
            for n in 0..frames_in {
                let s = (2.0 * std::f32::consts::PI * 440.0 * n as f32 / i as f32).sin() * 0.5;
                data.push(s);
                data.push(s);
            }
            let mut out = Vec::new();
            for block in data.chunks(1000 * 2) {
                c.process(block, &mut out);
            }
            c.finish(&mut out);
            let frames_out = out.len() / 2;
            assert_eq!(frames_out as u64, (frames_in as f64 * o as f64 / i as f64).round() as u64, "{i}->{o}");
            // l'amplitude doit être préservée (milieu du signal)
            let mid = &out[frames_out..frames_out + 2000];
            let peak = mid.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak > 0.45 && peak < 0.56, "peak={peak} {i}->{o}");
        }
    }
}
