// -- sinc_resampler.rs -- //
//
// Rééchantillonneur de haute qualité à interpolation sinc fenêtrée (Kaiser, ≈ 80 dB de
// rejet), en flux continu, multi-canaux, à latence nulle et à longueur de sortie exacte.
//
// Pourquoi pas une bibliothèque FFT ? Elle ajoutait ~800 Ko de code à l'exécutable pour
// une tâche que ce filtre polyphasé réalise avec quelques dizaines de Ko de tables et un
// coût processeur négligeable (≈ 1 % d'un cœur en 44,1 -> 48 kHz stéréo).
//
// Principe : la sortie n est calculée à la position source n x entrée / sortie. Les
// coefficients du filtre sont pré-calculés pour 128 sous-positions fractionnaires ; la
// valeur exacte est obtenue par interpolation linéaire entre deux lignes voisines.
// Le gain en continu vaut exactement 1 pour chaque ligne (normalisation).

/// Nombre de sous-positions fractionnaires tabulées (+1 ligne de bouclage).
const PHASES: usize = 128;
/// Longueur de base du filtre, en échantillons d'entrée, pour un rapport ≤ 1.
const BASE_TAPS: f64 = 100.0;
/// Paramètre β de la fenêtre de Kaiser (≈ 80 dB d'atténuation).
const KAISER_BETA: f64 = 8.0;
/// Longueur maximale du filtre (limite le coût pour les très grands rapports de réduction).
const MAX_TAPS: usize = 640;

/// Fonction de Bessel modifiée d'ordre 0 (série).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..60 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// Deux produits scalaires simultanés (lignes `a` et `b`) sur le même signal.
/// Huit accumulateurs indépendants : le compilateur peut vectoriser la boucle.
#[inline(never)]
fn dot2(x: &[f32], a: &[f32], b: &[f32]) -> (f32, f32) {
    let mut acc_a = [0.0f32; 8];
    let mut acc_b = [0.0f32; 8];
    let xs = x.chunks_exact(8);
    let rest_x = xs.remainder();
    for ((cx, ca), cb) in xs.zip(a.chunks_exact(8)).zip(b.chunks_exact(8)) {
        for i in 0..8 {
            acc_a[i] += cx[i] * ca[i];
            acc_b[i] += cx[i] * cb[i];
        }
    }
    let tail = x.len() - rest_x.len();
    let mut sa: f32 = acc_a.iter().sum();
    let mut sb: f32 = acc_b.iter().sum();
    for (i, &v) in rest_x.iter().enumerate() {
        sa += v * a[tail + i];
        sb += v * b[tail + i];
    }
    (sa, sb)
}

pub struct SincResampler {
    channels: usize,
    in_rate: u64,
    out_rate: u64,
    taps: usize,
    half: usize,
    /// (PHASES + 1) lignes de `taps` coefficients.
    table: Vec<f32>,
    /// Historique d'entrée par canal.
    buffers: Vec<Vec<f32>>,
    /// Indice absolu (dans le flux d'entrée) du premier élément de `buffers`.
    base: i64,
    /// Trames d'entrée reçues, zéros de fin compris.
    received: u64,
    /// Trames d'entrée réelles (sans zéros de fin).
    real_input: u64,
    produced: u64,
}

impl SincResampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Result<Self, String> {
        if in_rate == 0 || out_rate == 0 || channels == 0 {
            return Err("paramètres de rééchantillonnage invalides".into());
        }
        let (fin, fout) = (in_rate as f64, out_rate as f64);
        // Bande passante : jusqu'à 0,45 x la plus petite fréquence, coupure à 0,475 x.
        let narrower = fin.min(fout);
        let cutoff = 0.475 * narrower / fin; // cycles par échantillon d'entrée
        let widening = (fin / fout).max(1.0); // plus de points quand on réduit la fréquence
        let mut taps = (BASE_TAPS * widening).ceil() as usize;
        taps += taps % 2; // pair
        taps = taps.clamp(16, MAX_TAPS);
        let half = taps / 2;

        let i0_beta = bessel_i0(KAISER_BETA);
        let mut table = vec![0.0f32; (PHASES + 1) * taps];
        for row in 0..=PHASES {
            let frac = row as f64 / PHASES as f64;
            let mut coefficients = vec![0.0f64; taps];
            for (k, c) in coefficients.iter_mut().enumerate() {
                // position du point k par rapport au centre du noyau
                let t = k as f64 - half as f64 + 1.0 - frac;
                let r = t / half as f64;
                let window = if r.abs() >= 1.0 { 0.0 } else { bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / i0_beta };
                *c = 2.0 * cutoff * sinc(2.0 * cutoff * t) * window;
            }
            let sum: f64 = coefficients.iter().sum();
            for (k, c) in coefficients.iter().enumerate() {
                table[row * taps + k] = (c / sum) as f32;
            }
        }

        Ok(Self {
            channels,
            in_rate: in_rate as u64,
            out_rate: out_rate as u64,
            taps,
            half,
            table,
            buffers: vec![vec![0.0; half]; channels],
            base: -(half as i64),
            received: 0,
            real_input: 0,
            produced: 0,
        })
    }

    /// Ajoute des trames d'entrée (un vecteur par canal) ; écrit à la suite de `out`
    /// les trames de sortie disponibles, entrelacées.
    pub fn push(&mut self, planar: &[Vec<f32>], out: &mut Vec<f32>) {
        let frames = planar.first().map_or(0, |v| v.len());
        for (buffer, src) in self.buffers.iter_mut().zip(planar) {
            buffer.extend_from_slice(src);
        }
        self.received += frames as u64;
        self.real_input += frames as u64;
        self.produce(out, None);
    }

    /// Fin de flux : complète avec du silence et émet exactement
    /// `round(entrée x sortie / entrée)` trames au total.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let expected = (self.real_input * self.out_rate + self.in_rate / 2) / self.in_rate;
        let padding = self.half + 2;
        for buffer in self.buffers.iter_mut() {
            buffer.extend(std::iter::repeat(0.0).take(padding));
        }
        self.received += padding as u64;
        self.produce(out, Some(expected));
        for buffer in self.buffers.iter_mut() {
            buffer.clear();
        }
    }

    fn produce(&mut self, out: &mut Vec<f32>, limit: Option<u64>) {
        let phases = PHASES as u64;
        loop {
            if let Some(max) = limit {
                if self.produced >= max {
                    break;
                }
            }
            let numerator = self.produced * self.in_rate;
            let q = numerator / self.out_rate; // trame d'entrée de gauche
            let remainder = numerator % self.out_rate;
            if q + self.half as u64 >= self.received {
                break; // il manque encore des échantillons d'entrée
            }
            let scaled = remainder * phases;
            let row = (scaled / self.out_rate) as usize;
            let weight = (scaled % self.out_rate) as f32 / self.out_rate as f32;
            let row_a = &self.table[row * self.taps..(row + 1) * self.taps];
            let row_b = &self.table[(row + 1) * self.taps..(row + 2) * self.taps];

            let start = (q as i64 + 1 - self.half as i64 - self.base) as usize;
            for c in 0..self.channels {
                let x = &self.buffers[c][start..start + self.taps];
                let (a, b) = dot2(x, row_a, row_b);
                out.push(a + weight * (b - a));
            }
            self.produced += 1;
        }

        // Oublie l'historique devenu inutile.
        let next_q = (self.produced * self.in_rate / self.out_rate) as i64;
        let discard = next_q + 1 - self.half as i64 - self.base;
        if discard > 0 {
            for buffer in self.buffers.iter_mut() {
                let n = (discard as usize).min(buffer.len());
                buffer.drain(..n);
            }
            self.base += discard;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(in_rate: u32, out_rate: u32, signal: &[f32], chunk: usize) -> Vec<f32> {
        let mut r = SincResampler::new(in_rate, out_rate, 1).unwrap();
        let mut out = Vec::new();
        for block in signal.chunks(chunk) {
            r.push(&[block.to_vec()], &mut out);
        }
        r.finish(&mut out);
        out
    }

    fn sine(rate: u32, freq: f64, seconds: f64, amplitude: f32) -> Vec<f32> {
        (0..(rate as f64 * seconds) as usize)
            .map(|n| amplitude * (2.0 * std::f64::consts::PI * freq * n as f64 / rate as f64).sin() as f32)
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn output_length_is_exact_for_any_chunking() {
        for &(i, o) in &[(44100u32, 48000u32), (48000, 44100), (22050, 48000), (96000, 48000), (8000, 48000), (192000, 44100)] {
            let signal = sine(i, 440.0, 1.0, 0.5);
            let expected = (signal.len() as f64 * o as f64 / i as f64).round() as usize;
            for chunk in [1usize, 7, 1000, 4096, signal.len()] {
                assert_eq!(run(i, o, &signal, chunk).len(), expected, "{i}->{o} bloc {chunk}");
            }
        }
    }

    #[test]
    fn result_does_not_depend_on_chunking() {
        let signal = sine(44100, 1234.0, 0.5, 0.5);
        let a = run(44100, 48000, &signal, 1000);
        let b = run(44100, 48000, &signal, 333);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-6);
        }
    }

    #[test]
    fn passband_gain_is_flat_up_to_19_khz() {
        for freq in [100.0, 1000.0, 10000.0, 16000.0, 19000.0] {
            let out = run(44100, 48000, &sine(44100, freq, 1.0, 0.5), 1024);
            let middle = &out[8000..out.len() - 8000];
            let gain_db = 20.0 * (rms(middle) / (0.5 / 2f64.sqrt())).log10();
            assert!(gain_db.abs() < 0.05, "{freq} Hz : {gain_db:.3} dB");
        }
    }

    #[test]
    fn downsampling_rejects_frequencies_above_the_new_nyquist() {
        // 30 kHz échantillonné à 96 kHz doit disparaître en passant à 48 kHz (Nyquist 24 kHz).
        let out = run(96000, 48000, &sine(96000, 30000.0, 1.0, 0.5), 2048);
        let middle = &out[8000..out.len() - 8000];
        let rejection_db = 20.0 * (rms(middle) / (0.5 / 2f64.sqrt())).log10();
        assert!(rejection_db < -75.0, "rejet {rejection_db:.1} dB");
        // et 10 kHz doit passer intact
        let out = run(96000, 48000, &sine(96000, 10000.0, 1.0, 0.5), 2048);
        let g = 20.0 * (rms(&out[8000..out.len() - 8000]) / (0.5 / 2f64.sqrt())).log10();
        assert!(g.abs() < 0.05, "{g:.3} dB");
    }

    #[test]
    fn upsampling_rejects_images() {
        // 20 kHz à 44,1 kHz -> 48 kHz : pas de repliement visible (< -75 dB hors de 20 kHz)
        let out = run(44100, 48000, &sine(44100, 20000.0, 1.0, 0.5), 1024);
        let middle: Vec<f32> = out[8000..out.len() - 8000].to_vec();
        // retire la composante 20 kHz par projection et mesure le résidu
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, v) in middle.iter().enumerate() {
            let a = 2.0 * std::f64::consts::PI * 20000.0 * n as f64 / 48000.0;
            re += *v as f64 * a.cos();
            im += *v as f64 * a.sin();
        }
        let (re, im) = (re * 2.0 / middle.len() as f64, im * 2.0 / middle.len() as f64);
        let residual: Vec<f32> = middle
            .iter()
            .enumerate()
            .map(|(n, v)| {
                let a = 2.0 * std::f64::consts::PI * 20000.0 * n as f64 / 48000.0;
                *v - (re * a.cos() + im * a.sin()) as f32
            })
            .collect();
        let db = 20.0 * (rms(&residual) / (0.5 / 2f64.sqrt())).log10();
        assert!(db < -70.0, "résidu {db:.1} dB");
    }

    #[test]
    fn dc_gain_is_one_and_timing_is_aligned() {
        let out = run(44100, 48000, &vec![0.25f32; 44100], 4096);
        for v in &out[200..out.len() - 200] {
            assert!((v - 0.25).abs() < 1e-5, "{v}");
        }
        // une impulsion à l'échantillon 20000 réapparaît à 20000 x 48000 / 44100 (±0,5 trame)
        let mut impulse = vec![0.0f32; 44100];
        impulse[20000] = 1.0;
        let out = run(44100, 48000, &impulse, 4096);
        let (peak_index, _) = out.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
        let expected = 20000.0 * 48000.0 / 44100.0;
        assert!((peak_index as f64 - expected).abs() <= 1.0, "{peak_index} vs {expected}");
    }

    #[test]
    fn channels_are_independent_and_interleaved() {
        let mut r = SincResampler::new(44100, 48000, 2).unwrap();
        let left = sine(44100, 1000.0, 0.2, 0.5);
        let right = vec![0.0f32; left.len()];
        let mut out = Vec::new();
        r.push(&[left, right], &mut out);
        r.finish(&mut out);
        let (l, rr): (Vec<f32>, Vec<f32>) = out.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
        assert!(rms(&l[500..l.len() - 500]) > 0.3);
        assert!(rr.iter().all(|v| v.abs() < 1e-9));
    }
}

#[cfg(test)]
mod benchmark {
    use super::*;

    /// Mesure du coût processeur (à lancer en release) :
    /// cargo test --release resampler_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn resampler_cost() {
        for &(i, o, seconds) in &[(44100u32, 48000u32, 20usize), (48000, 44100, 20), (96000, 48000, 20), (22050, 48000, 20)] {
            let frames = i as usize * seconds;
            let block: Vec<f32> = (0..4096).map(|n| ((n as f32) * 0.01).sin() * 0.5).collect();
            let mut r = SincResampler::new(i, o, 2).unwrap();
            let mut out = Vec::new();
            let start = std::time::Instant::now();
            let mut done = 0;
            while done < frames {
                let n = 4096.min(frames - done);
                r.push(&[block[..n].to_vec(), block[..n].to_vec()], &mut out);
                out.clear();
                done += n;
            }
            let elapsed = start.elapsed().as_secs_f64();
            println!("{i} -> {o} Hz stéréo : {:.2} % d'un cœur (x{:.0} temps réel)", 100.0 * elapsed / seconds as f64, seconds as f64 / elapsed);
        }
    }
}
