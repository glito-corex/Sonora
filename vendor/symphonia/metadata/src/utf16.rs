// Sonora : décodage UTF-16 minimal (remplace encoding_rs, dont les tables de plusieurs
// dizaines de langues alourdissaient l'exécutable pour des étiquettes que Sonora ne lit pas).

/// Décode de l'UTF-16 en UTF-8. Si une marque d'ordre des octets (BOM) est présente elle
/// indique l'ordre ; sinon l'ordre gros-boutiste est supposé.
pub fn decode_utf16_with_bom(data: &[u8]) -> String {
    let (little_endian, body) = match data {
        [0xFF, 0xFE, rest @ ..] => (true, rest),
        [0xFE, 0xFF, rest @ ..] => (false, rest),
        _ => (false, data),
    };
    let units = body.chunks_exact(2).map(|b| {
        if little_endian { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) }
    });
    char::decode_utf16(units).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

#[cfg(test)]
mod tests {
    use super::decode_utf16_with_bom;

    #[test]
    fn decodes_with_and_without_bom() {
        assert_eq!(decode_utf16_with_bom(&[0x00, 0x48, 0x00, 0x69]), "Hi");
        assert_eq!(decode_utf16_with_bom(&[0xFE, 0xFF, 0x00, 0x48]), "H");
        assert_eq!(decode_utf16_with_bom(&[0xFF, 0xFE, 0x48, 0x00, 0xE9, 0x00]), "Hé");
    }
}
