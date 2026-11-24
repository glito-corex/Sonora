// -- opus_decoder.rs -- //
//
// Décodeur Opus pour Symphonia, basé sur libopus (liée statiquement).
// Symphonia fournit les paquets (ogg / mkv / webm / mp4) et l'en-tête OpusHead
// (dans `extra_data`) ; ce module ne fait que décoder.
//
// Ce code n'est exécuté que pour une piste Opus : le registre ne crée ce
// décodeur que lorsque le codec de la piste choisie est Opus.

use std::borrow::Cow;

use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Channels, Signal, SignalSpec};
use symphonia::core::codecs::{
    CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult, CODEC_TYPE_OPUS,
};
use symphonia::core::errors::{decode_error, unsupported_error, Result};
use symphonia::core::formats::Packet;
use symphonia::core::support_codec;

use crate::libopus_bindings::{MultistreamDecoder, MAX_FRAME_SAMPLES, OPUS_SAMPLE_RATE};

/// Contenu utile de l'en-tête OpusHead (RFC 7845, section 5.1).
#[derive(Debug, Clone, PartialEq)]
pub struct OpusHead {
    pub channels: usize,
    pub pre_skip: usize,
    /// Gain de sortie en Q7.8 dB.
    pub output_gain: i16,
    pub streams: usize,
    pub coupled_streams: usize,
    pub mapping: Vec<u8>,
    pub mapping_family: u8,
}

pub fn parse_opus_head(data: &[u8]) -> std::result::Result<OpusHead, &'static str> {
    if data.len() < 19 || &data[..8] != b"OpusHead" {
        return Err("en-tête OpusHead absent ou invalide");
    }
    // Version 0 = boîte « dOps » de mp4 (champs en gros-boutiste, ISO/IEC 14496-12
    // Opus) reconstruite par le démuxeur ; version 1 = OpusHead ogg/mkv (petit-boutiste).
    let version = data[8];
    if version >> 4 != 0 {
        return Err("version d'en-tête Opus non supportée");
    }
    let big_endian = version == 0;
    let read_u16 = |i: usize| -> u16 {
        let b = [data[i], data[i + 1]];
        if big_endian { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    let channels = data[9] as usize;
    if channels == 0 {
        return Err("Opus : nombre de canaux nul");
    }
    let pre_skip = read_u16(10) as usize;
    let output_gain = read_u16(16) as i16;
    let family = data[18];

    let (streams, coupled, mapping) = match family {
        0 => {
            if channels > 2 {
                return Err("Opus famille 0 : 1 ou 2 canaux seulement");
            }
            (1usize, if channels == 2 { 1usize } else { 0usize }, (0..channels as u8).collect::<Vec<u8>>())
        }
        1 => {
            if data.len() < 21 + channels {
                return Err("table de canaux Opus tronquée");
            }
            let s = data[19] as usize;
            let c = data[20] as usize;
            (s, c, data[21..21 + channels].to_vec())
        }
        _ => return Err("disposition de canaux Opus non supportée (famille 2/255)"),
    };

    Ok(OpusHead {
        channels,
        pre_skip,
        output_gain,
        streams,
        coupled_streams: coupled,
        mapping,
        mapping_family: family,
    })
}

/// Canaux Symphonia et permutation depuis l'ordre Vorbis utilisé par Opus
/// (`perm[i]` = indice source du i-ème canal dans l'ordre de bits Symphonia).
fn channel_layout(channels: usize) -> Option<(Channels, Vec<usize>)> {
    use Channels as C;
    let fl = C::FRONT_LEFT;
    let fr = C::FRONT_RIGHT;
    let fc = C::FRONT_CENTRE;
    let lfe = C::LFE1;
    let rl = C::REAR_LEFT;
    let rr = C::REAR_RIGHT;
    let rc = C::REAR_CENTRE;
    let sl = C::SIDE_LEFT;
    let sr = C::SIDE_RIGHT;
    Some(match channels {
        1 => (fl, vec![0]),
        2 => (fl | fr, vec![0, 1]),
        3 => (fl | fr | fc, vec![0, 2, 1]),
        4 => (fl | fr | rl | rr, vec![0, 1, 2, 3]),
        5 => (fl | fr | fc | rl | rr, vec![0, 2, 1, 3, 4]),
        6 => (fl | fr | fc | lfe | rl | rr, vec![0, 2, 1, 5, 3, 4]),
        7 => (fl | fr | fc | lfe | rc | sl | sr, vec![0, 2, 1, 6, 5, 3, 4]),
        8 => (fl | fr | fc | lfe | rl | rr | sl | sr, vec![0, 2, 1, 7, 5, 6, 3, 4]),
        _ => return None,
    })
}

pub struct OpusDecoder {
    params: CodecParameters,
    inner: MultistreamDecoder,
    channels: usize,
    perm: Vec<usize>,
    pcm: Vec<f32>,
    buf: AudioBuffer<f32>,
    /// Échantillons d'amorce (pre-skip) de l'en-tête, à écarter au début du flux.
    pre_skip: usize,
    skip_left: usize,
}

impl Decoder for OpusDecoder {
    fn try_new(params: &CodecParameters, _options: &DecoderOptions) -> Result<Self> {
        if params.codec != CODEC_TYPE_OPUS {
            return unsupported_error("opus: type de codec invalide");
        }
        let Some(extra) = params.extra_data.as_deref() else {
            return unsupported_error("opus: en-tête OpusHead manquant");
        };
        let head = match parse_opus_head(extra) {
            Ok(h) => h,
            Err(e) => return unsupported_error(e),
        };
        let Some((layout, perm)) = channel_layout(head.channels) else {
            return unsupported_error("opus: nombre de canaux non supporté");
        };

        let mut inner = match MultistreamDecoder::new(head.channels, head.streams, head.coupled_streams, &head.mapping) {
            Ok(d) => d,
            Err(_) => return unsupported_error("opus: création du décodeur impossible"),
        };
        if head.output_gain != 0 {
            let _ = inner.set_gain_q8(head.output_gain as i32);
        }

        let spec = SignalSpec::new(OPUS_SAMPLE_RATE as u32, layout);
        let mut params = params.clone();
        params.with_sample_rate(OPUS_SAMPLE_RATE as u32).with_channels(layout);

        // Aucun démuxeur ne retire l'amorce (pre-skip) lui-même : c'est le décodeur qui l'écarte.
        let pre_skip = head.pre_skip;

        Ok(OpusDecoder {
            params,
            inner,
            channels: head.channels,
            perm,
            pcm: vec![0.0; MAX_FRAME_SAMPLES * head.channels],
            buf: AudioBuffer::new(MAX_FRAME_SAMPLES as u64, spec),
            pre_skip,
            skip_left: pre_skip,
        })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[support_codec!(CODEC_TYPE_OPUS, "opus", "Opus (libopus)")]
    }

    fn reset(&mut self) {
        self.inner.reset();
        // Après un déplacement on est au milieu du flux : plus d'amorce à écarter.
        self.skip_left = 0;
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        let data = packet.buf();
        let frames = match self.inner.decode_float(data, &mut self.pcm, MAX_FRAME_SAMPLES) {
            Ok(n) => n,
            Err(_) => return decode_error("opus: paquet invalide"),
        };

        self.buf.clear();
        self.buf.render_reserved(Some(frames));
        let ch = self.channels;
        for (dst_index, &src) in self.perm.iter().enumerate() {
            let plane = self.buf.chan_mut(dst_index);
            for f in 0..frames {
                plane[f] = self.pcm[f * ch + src];
            }
        }

        // Écarte l'amorce du début de flux.
        if self.skip_left > 0 {
            let s = self.skip_left.min(frames);
            self.skip_left -= s;
            self.buf.trim(s, 0);
        }

        Ok(AudioBufferRef::F32(Cow::Borrowed(&self.buf)))
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        AudioBufferRef::F32(Cow::Borrowed(&self.buf))
    }
}

impl OpusDecoder {
    #[allow(dead_code)]
    pub fn pre_skip(&self) -> usize {
        self.pre_skip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(ch: u8, family: u8, extra: &[u8]) -> Vec<u8> {
        let mut v = b"OpusHead".to_vec();
        v.extend_from_slice(&[1, ch, 0x38, 0x01, 0x80, 0xbb, 0, 0, 0, 0, family]);
        v.extend_from_slice(extra);
        v
    }

    #[test]
    fn parses_stereo_head() {
        let h = parse_opus_head(&head(2, 0, &[])).unwrap();
        assert_eq!((h.channels, h.pre_skip, h.streams, h.coupled_streams), (2, 312, 1, 1));
        assert_eq!(h.mapping, vec![0, 1]);
    }

    #[test]
    fn parses_mp4_dops_big_endian_head() {
        // version 0 (dOps) : pre-skip 312 écrit en gros-boutiste
        let mut v = b"OpusHead".to_vec();
        v.extend_from_slice(&[0, 2, 0x01, 0x38, 0x00, 0x00, 0xbb, 0x80, 0x00, 0x00, 0x00]);
        let h = parse_opus_head(&v).unwrap();
        assert_eq!((h.channels, h.pre_skip, h.output_gain), (2, 312, 0));
    }

    #[test]
    fn parses_surround_head() {
        let h = parse_opus_head(&head(6, 1, &[4, 2, 0, 4, 1, 2, 3, 5])).unwrap();
        assert_eq!((h.channels, h.streams, h.coupled_streams), (6, 4, 2));
        assert_eq!(h.mapping, vec![0, 4, 1, 2, 3, 5]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_opus_head(b"nope").is_err());
        assert!(parse_opus_head(&head(0, 0, &[])).is_err());
        assert!(parse_opus_head(&head(3, 0, &[])).is_err());
        assert!(parse_opus_head(&head(2, 255, &[])).is_err());
    }

    #[test]
    fn channel_layouts_have_matching_counts() {
        for n in 1..=8 {
            let (layout, perm) = channel_layout(n).unwrap();
            assert_eq!(layout.count(), n);
            let mut sorted = perm.clone();
            sorted.sort();
            assert_eq!(sorted, (0..n).collect::<Vec<_>>());
        }
        assert!(channel_layout(9).is_none());
    }
}
