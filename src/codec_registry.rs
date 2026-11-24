// -- codec_registry.rs -- //
//
// Registre des décodeurs : ceux de Symphonia activés + Opus (libopus).
// Un décodeur n'est instancié que pour la piste réellement lue : lire un
// fichier Opus n'exécute aucun code AAC/MP3/Vorbis, et inversement.

use std::sync::OnceLock;

use symphonia::core::codecs::CodecRegistry;

use crate::opus_decoder::OpusDecoder;

pub fn codec_registry() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_all::<OpusDecoder>();
        registry
    })
}
