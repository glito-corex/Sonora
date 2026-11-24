// -- libopus_bindings.rs -- //
//
// Liaison minimale avec libopus : UNIQUEMENT les fonctions de décodage
// multi-flux (qui couvrent aussi le mono/stéréo). Aucune fonction d'encodage
// n'est déclarée ici : la bibliothèque compilée par build.rs n'en contient pas.

use std::os::raw::{c_int, c_uchar};

#[repr(C)]
pub struct OpusMSDecoder {
    _private: [u8; 0],
}

extern "C" {
    fn opus_multistream_decoder_create(
        fs: i32,
        channels: c_int,
        streams: c_int,
        coupled_streams: c_int,
        mapping: *const c_uchar,
        error: *mut c_int,
    ) -> *mut OpusMSDecoder;

    fn opus_multistream_decode_float(
        st: *mut OpusMSDecoder,
        data: *const c_uchar,
        len: i32,
        pcm: *mut f32,
        frame_size: c_int,
        decode_fec: c_int,
    ) -> c_int;

    fn opus_multistream_decoder_ctl(st: *mut OpusMSDecoder, request: c_int, ...) -> c_int;

    fn opus_multistream_decoder_destroy(st: *mut OpusMSDecoder);

    fn opus_strerror(error: c_int) -> *const std::os::raw::c_char;
}

const OPUS_SET_GAIN_REQUEST: c_int = 4034;
const OPUS_RESET_STATE: c_int = 4028;

/// Durée maximale d'une trame Opus : 120 ms à 48 kHz.
pub const MAX_FRAME_SAMPLES: usize = 5760;
pub const OPUS_SAMPLE_RATE: i32 = 48_000;

/// Décodeur libopus (propriétaire du pointeur C, libéré dans `Drop`).
pub struct MultistreamDecoder {
    raw: *mut OpusMSDecoder,
    channels: usize,
}

// libopus n'a pas d'état global : un décodeur peut changer de thread tant qu'il
// n'est pas utilisé en parallèle, ce que garantit `&mut self`.
unsafe impl Send for MultistreamDecoder {}
unsafe impl Sync for MultistreamDecoder {}

pub fn error_message(code: c_int) -> String {
    unsafe {
        let p = opus_strerror(code);
        if p.is_null() {
            return format!("erreur libopus {code}");
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

impl MultistreamDecoder {
    pub fn new(
        channels: usize,
        streams: usize,
        coupled_streams: usize,
        mapping: &[u8],
    ) -> Result<Self, String> {
        if mapping.len() != channels {
            return Err("table de correspondance des canaux invalide".into());
        }
        let mut error: c_int = 0;
        let raw = unsafe {
            opus_multistream_decoder_create(
                OPUS_SAMPLE_RATE,
                channels as c_int,
                streams as c_int,
                coupled_streams as c_int,
                mapping.as_ptr(),
                &mut error,
            )
        };
        if raw.is_null() || error != 0 {
            return Err(error_message(error));
        }
        Ok(Self { raw, channels })
    }

    /// Gain de sortie en Q7.8 dB (champ « output gain » de l'en-tête OpusHead).
    pub fn set_gain_q8(&mut self, gain: i32) -> Result<(), String> {
        let r = unsafe { opus_multistream_decoder_ctl(self.raw, OPUS_SET_GAIN_REQUEST, gain as c_int) };
        if r != 0 {
            return Err(error_message(r));
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        unsafe {
            opus_multistream_decoder_ctl(self.raw, OPUS_RESET_STATE);
        }
    }

    /// Décode un paquet vers `pcm` (entrelacé, `channels` × trames). Renvoie le nombre de trames.
    /// `data` vide = paquet perdu (reconstruction PLC de `frame_size` trames).
    pub fn decode_float(&mut self, data: &[u8], pcm: &mut [f32], frame_size: usize) -> Result<usize, String> {
        if pcm.len() < frame_size * self.channels {
            return Err("tampon PCM trop petit".into());
        }
        let ptr = if data.is_empty() { std::ptr::null() } else { data.as_ptr() };
        let n = unsafe {
            opus_multistream_decode_float(
                self.raw,
                ptr,
                data.len() as i32,
                pcm.as_mut_ptr(),
                frame_size as c_int,
                0,
            )
        };
        if n < 0 {
            Err(error_message(n))
        } else {
            Ok(n as usize)
        }
    }
}

impl Drop for MultistreamDecoder {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { opus_multistream_decoder_destroy(self.raw) };
        }
    }
}
