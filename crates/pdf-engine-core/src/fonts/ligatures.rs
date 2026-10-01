//! Typographic ligature decomposition and recomposition.
//!
//! Handles standard Latin and OpenType typographic ligatures (`fi`, `fl`, `ffi`, `ffl`, `st`)
//! so that text editing operations operate on canonical Unicode characters without
//! leaving dangling or corrupted composite glyphs.

/// Decomposes typographic ligatures into their canonical constituent characters.
pub fn decompose_ligatures(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' => out.push_str("st"),
            '\u{FB06}' => out.push_str("st"),
            other => out.push(other),
        }
    }
    out
}

/// Recomposes character sequences into standard typographic ligatures when supported.
pub fn compose_ligatures(input: &str) -> String {
    let mut out = input.to_string();
    out = out.replace("ffi", "\u{FB03}");
    out = out.replace("ffl", "\u{FB04}");
    out = out.replace("ff", "\u{FB00}");
    out = out.replace("fi", "\u{FB01}");
    out = out.replace("fl", "\u{FB02}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ligature_decomposition() {
        let text_with_ligatures = "The \u{FB01}nal \u{FB02}ow and o\u{FB03}ce";
        let decomposed = decompose_ligatures(text_with_ligatures);
        assert_eq!(decomposed, "The final flow and office");
    }

    #[test]
    fn test_ligature_recomposition() {
        let text = "final flow office";
        let composed = compose_ligatures(text);
        assert_eq!(composed, "\u{FB01}nal \u{FB02}ow o\u{FB03}ce");
    }
}
