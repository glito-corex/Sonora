// -- build.rs -- //
//
// 1) Compile le DÉCODEUR de libopus (sources dans vendor/libopus) et le lie
//    STATIQUEMENT dans l'exécutable : aucune DLL.
//    Seuls les 65 fichiers nécessaires au décodage sont compilés : tout
//    l'encodeur (opus_encoder, celt_encoder, SILK encode/NSQ/VAD, analyse...)
//    n'est jamais compilé. Chaque fonction est dans sa propre section, et
//    l'éditeur de liens retire tout ce que le décodeur ne référence pas.
// 2) Dessine l'icône de Sonora EN CODE (aucun fichier image dans le projet),
//    l'écrit au format .ico (PNG de 16 à 256 px) et l'intègre à l'exécutable
//    avec le manifeste et les informations de version : c'est l'icône de la
//    barre de titre, de la barre des tâches et de l'Explorateur.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const OPUS_DECODER_SOURCES: &[&str] = &[
    "celt/bands.c",
    "celt/celt.c",
    "celt/celt_decoder.c",
    "celt/celt_lpc.c",
    "celt/cwrs.c",
    "celt/entcode.c",
    "celt/entdec.c",
    "celt/entenc.c",
    "celt/kiss_fft.c",
    "celt/laplace.c",
    "celt/mathops.c",
    "celt/mdct.c",
    "celt/modes.c",
    "celt/pitch.c",
    "celt/quant_bands.c",
    "celt/rate.c",
    "celt/vq.c",
    "silk/CNG.c",
    "silk/LPC_analysis_filter.c",
    "silk/LPC_fit.c",
    "silk/LPC_inv_pred_gain.c",
    "silk/NLSF2A.c",
    "silk/NLSF_decode.c",
    "silk/NLSF_stabilize.c",
    "silk/NLSF_unpack.c",
    "silk/PLC.c",
    "silk/bwexpander.c",
    "silk/bwexpander_32.c",
    "silk/code_signs.c",
    "silk/dec_API.c",
    "silk/decode_core.c",
    "silk/decode_frame.c",
    "silk/decode_indices.c",
    "silk/decode_parameters.c",
    "silk/decode_pitch.c",
    "silk/decode_pulses.c",
    "silk/decoder_set_fs.c",
    "silk/gain_quant.c",
    "silk/init_decoder.c",
    "silk/lin2log.c",
    "silk/log2lin.c",
    "silk/pitch_est_tables.c",
    "silk/resampler.c",
    "silk/resampler_private_AR2.c",
    "silk/resampler_private_IIR_FIR.c",
    "silk/resampler_private_down_FIR.c",
    "silk/resampler_private_up2_HQ.c",
    "silk/resampler_rom.c",
    "silk/shell_coder.c",
    "silk/sort.c",
    "silk/stereo_MS_to_LR.c",
    "silk/stereo_decode_pred.c",
    "silk/sum_sqr_shift.c",
    "silk/table_LSF_cos.c",
    "silk/tables_LTP.c",
    "silk/tables_NLSF_CB_NB_MB.c",
    "silk/tables_NLSF_CB_WB.c",
    "silk/tables_gain.c",
    "silk/tables_other.c",
    "silk/tables_pitch_lag.c",
    "silk/tables_pulses_per_block.c",
    "src/opus.c",
    "src/opus_decoder.c",
    "src/opus_multistream.c",
    "src/opus_multistream_decoder.c",
];

/// Tailles (px) contenues dans l'icône : 100 % à 200 % de mise à l'échelle, jusqu'à 256 px.
const ICON_SIZES: &[usize] = &[16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/libopus");
    println!("cargo:rerun-if-changed=resources");

    build_libopus_decoder();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
    }
}

fn build_libopus_decoder() {
    let root = PathBuf::from("vendor/libopus");
    let mut build = cc::Build::new();
    build
        .include(root.join("include"))
        .include(root.join("celt"))
        .include(root.join("silk"))
        .include(root.join("src"))
        .define("OPUS_BUILD", None)
        .define("USE_ALLOCA", None)
        .define("HAVE_LRINT", None)
        .define("HAVE_LRINTF", None)
        .warnings(false)
        .opt_level(3)
        .flag_if_supported("-ffunction-sections")
        .flag_if_supported("-fdata-sections")
        .flag_if_supported("-fvisibility=hidden");
    for f in OPUS_DECODER_SOURCES {
        build.file(root.join(f));
    }
    build.compile("opus_decoder");
}

// ---------------------------------------------------------------------------
// Ressources Windows : icône dessinée en code, manifeste, informations de version
// ---------------------------------------------------------------------------

fn embed_windows_resources() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    let icon_path = out_dir.join("sonora.ico");
    std::fs::write(&icon_path, build_ico()).expect("écriture de l'icône");

    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0));

    let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let mut rc = String::new();
    let _ = writeln!(rc, "#define RT_MANIFEST 24");
    let _ = writeln!(rc, "1 ICON \"{}\"", slash(&icon_path));
    let _ = writeln!(rc, "1 RT_MANIFEST \"{}\"", slash(&manifest_dir.join("resources/application.manifest")));
    let _ = writeln!(rc, "1 VERSIONINFO");
    let _ = writeln!(rc, "FILEVERSION {major},{minor},{patch},0");
    let _ = writeln!(rc, "PRODUCTVERSION {major},{minor},{patch},0");
    let _ = writeln!(rc, "FILEFLAGSMASK 0x3fL");
    let _ = writeln!(rc, "FILEFLAGS 0x0L");
    let _ = writeln!(rc, "FILEOS 0x40004L");
    let _ = writeln!(rc, "FILETYPE 0x1L");
    let _ = writeln!(rc, "FILESUBTYPE 0x0L");
    let _ = writeln!(rc, "BEGIN");
    let _ = writeln!(rc, "  BLOCK \"StringFileInfo\"");
    let _ = writeln!(rc, "  BEGIN");
    let _ = writeln!(rc, "    BLOCK \"040c04b0\"");
    let _ = writeln!(rc, "    BEGIN");
    for (key, value) in [
        ("FileDescription", "Sonora".to_string()),
        ("ProductName", "Sonora".to_string()),
        ("InternalName", "sonora".to_string()),
        ("OriginalFilename", "sonora.exe".to_string()),
        ("FileVersion", format!("{major}.{minor}.{patch}.0")),
        ("ProductVersion", format!("{major}.{minor}.{patch}.0")),
    ] {
        let _ = writeln!(rc, "      VALUE \"{key}\", \"{value}\\0\"");
    }
    let _ = writeln!(rc, "    END");
    let _ = writeln!(rc, "  END");
    let _ = writeln!(rc, "  BLOCK \"VarFileInfo\"");
    let _ = writeln!(rc, "  BEGIN");
    let _ = writeln!(rc, "    VALUE \"Translation\", 0x040c, 1200");
    let _ = writeln!(rc, "  END");
    let _ = writeln!(rc, "END");

    let rc_path = out_dir.join("sonora.rc");
    std::fs::write(&rc_path, rc).expect("écriture du fichier de ressources");
    embed_resource::compile(&rc_path, embed_resource::NONE);
}

// ---- dessin de l'icône (équivalent du dessin vectoriel d'origine) -----------------

type Rgba = [f32; 4]; // composantes pré-multipliées par l'alpha

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn hex(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
}

/// Dépose une couleur (alpha `a`) par-dessus `dst` (opérateur « over », pré-multiplié).
fn over(dst: &mut Rgba, color: [f32; 3], a: f32) {
    let inv = 1.0 - a;
    dst[0] = color[0] * a + dst[0] * inv;
    dst[1] = color[1] * a + dst[1] * inv;
    dst[2] = color[2] * a + dst[2] * inv;
    dst[3] = a + dst[3] * inv;
}

/// Dégradé diagonal « disc » (#38E6D0 -> #3685FF) sur la boîte englobante [min, min+size].
fn disc_gradient(x: f32, y: f32, min: f32, size: f32) -> [f32; 3] {
    let t = (((x - min) / size) + ((y - min) / size)) * 0.5;
    mix(hex(0x38, 0xE6, 0xD0), hex(0x36, 0x85, 0xFF), t)
}

/// Couleur (pré-multipliée) du point (x, y) de l'icône, dans un repère 256 x 256.
fn shade(x: f32, y: f32) -> Rgba {
    let mut px: Rgba = [0.0; 4];

    // fond : carré arrondi (rayon 58), dégradé diagonal #101E35 -> #080D19
    let cx = x.clamp(58.0, 198.0);
    let cy = y.clamp(58.0, 198.0);
    if (x - cx) * (x - cx) + (y - cy) * (y - cy) <= 58.0 * 58.0 {
        let c = mix(hex(0x10, 0x1E, 0x35), hex(0x08, 0x0D, 0x19), (x / 256.0 + y / 256.0) * 0.5);
        over(&mut px, c, 1.0);
    }

    let d = ((x - 128.0) * (x - 128.0) + (y - 128.0) * (y - 128.0)).sqrt();

    // anneau extérieur (r 78, trait 5, opacité 0,35)
    if (d - 78.0).abs() <= 2.5 {
        over(&mut px, disc_gradient(x, y, 50.0, 156.0), 0.35);
    }
    // disque (r 65) rempli #17283E, contour dégradé de 7
    if d <= 65.0 {
        over(&mut px, hex(0x17, 0x28, 0x3E), 1.0);
    }
    if (d - 65.0).abs() <= 3.5 {
        over(&mut px, disc_gradient(x, y, 63.0, 130.0), 1.0);
    }
    // anneaux fins turquoise
    if (d - 48.0).abs() <= 1.0 {
        over(&mut px, hex(0x42, 0xCF, 0xCB), 0.5);
    }
    if (d - 30.0).abs() <= 1.5 {
        over(&mut px, hex(0x42, 0xCF, 0xCB), 0.7);
    }
    // centre dégradé (r 13) et point blanc (r 5)
    if d <= 13.0 {
        over(&mut px, disc_gradient(x, y, 115.0, 26.0), 1.0);
    }
    if d <= 5.0 {
        over(&mut px, [1.0, 1.0, 1.0], 1.0);
    }
    px
}

/// Rend l'icône en RGBA 8 bits (alpha droit), sur-échantillonnée 4 x 4 pour un contour lisse.
fn render_icon(size: usize) -> Vec<u8> {
    const SAMPLES: usize = 4;
    let scale = 256.0 / size as f32;
    let mut out = vec![0u8; size * size * 4];
    for py in 0..size {
        for pxl in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (pxl as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * scale;
                    let c = shade(x, y);
                    for k in 0..4 {
                        acc[k] += c[k];
                    }
                }
            }
            let n = (SAMPLES * SAMPLES) as f32;
            let a = acc[3] / n;
            let o = (py * size + pxl) * 4;
            if a > 0.0 {
                for k in 0..3 {
                    out[o + k] = ((acc[k] / n / a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
                out[o + 3] = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

// ---- encodage PNG + ICO -----------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, slot) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

fn encode_png(size: usize, rgba: &[u8]) -> Vec<u8> {
    // filtre « Sub » sur chaque ligne : les dégradés lisses se compressent très bien
    let stride = size * 4;
    let mut raw = Vec::with_capacity((stride + 1) * size);
    for row in 0..size {
        raw.push(1);
        let line = &rgba[row * stride..(row + 1) * stride];
        for i in 0..stride {
            let left = if i >= 4 { line[i - 4] } else { 0 };
            raw.push(line[i].wrapping_sub(left));
        }
    }
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(size as u32).to_be_bytes());
    ihdr.extend_from_slice(&(size as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA
    png_chunk(&mut png, b"IHDR", &ihdr);
    png_chunk(&mut png, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 10));
    png_chunk(&mut png, b"IEND", &[]);
    png
}

fn build_ico() -> Vec<u8> {
    let images: Vec<(usize, Vec<u8>)> =
        ICON_SIZES.iter().map(|&s| (s, encode_png(s, &render_icon(s)))).collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (size, data) in &images {
        let dim = if *size >= 256 { 0u8 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // plans
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits par pixel
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in &images {
        ico.extend_from_slice(data);
    }
    ico
}
