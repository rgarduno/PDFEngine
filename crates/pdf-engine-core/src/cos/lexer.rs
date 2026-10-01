//! ISO 32000-1 §7.2 Lexical Scanner (Tokenizer).
//!
//! Provides zero-copy byte slice tokenization for PDF documents.
//! Accurately handles whitespace, comments, escape sequences, balanced literal strings,
//! hex strings, and name `#XX` character normalization.

use crate::cos::object::{PdfName, PdfString};
use crate::error::{PdfError, PdfResult};

/// Lexical tokens recognized in PDF document streams.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Boolean literal `true` or `false`.
    Boolean(bool),
    /// Integer numeric value.
    Integer(i64),
    /// Floating point numeric value.
    Real(f64),
    /// Name token `/Name` (normalized without leading slash).
    Name(PdfName),
    /// Literal string enclosed in `(...)`.
    LiteralString(PdfString),
    /// Hexadecimal string enclosed in `<...>`.
    HexString(PdfString),
    /// Null object token `null`.
    Null,
    /// Array opening delimiter `[`.
    ArrayStart,
    /// Array closing delimiter `]`.
    ArrayEnd,
    /// Dictionary opening delimiter `<<`.
    DictionaryStart,
    /// Dictionary closing delimiter `>>`.
    DictionaryEnd,
    /// Indirect reference designator `R`.
    R,
    /// Object definition start keyword `obj`.
    Obj,
    /// Object definition end keyword `endobj`.
    EndObj,
    /// Stream data start keyword `stream`.
    Stream,
    /// Stream data end keyword `endstream`.
    EndStream,
    /// Cross-reference table start keyword `xref`.
    XRef,
    /// Trailer dictionary keyword `trailer`.
    Trailer,
    /// Start of cross-reference offset keyword `startxref`.
    StartXRef,
}

/// Lexical scanner for PDF byte buffers.
pub struct Lexer<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> Lexer<'a> {
    /// Creates a new lexer over the provided byte slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }

    /// Creates a lexer starting from an explicit byte offset.
    pub fn at_offset(data: &'a [u8], offset: usize) -> Self {
        Self {
            data,
            cursor: offset.min(data.len()),
        }
    }

    /// Returns the current read offset within the buffer.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Sets the current read offset.
    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.data.len());
    }

    /// Returns true if the cursor has reached the end of the input buffer.
    pub fn is_eof(&self) -> bool {
        self.cursor >= self.data.len()
    }

    /// Returns the remaining unconsumed slice from the current cursor.
    pub fn remaining(&self) -> &'a [u8] {
        if self.cursor < self.data.len() {
            &self.data[self.cursor..]
        } else {
            &[]
        }
    }

    /// Checks if a byte is considered white space according to ISO 32000-1 Table 1.
    /// (NUL, HT, LF, FF, CR, SP).
    #[inline]
    pub fn is_whitespace(b: u8) -> bool {
        matches!(b, 0x00 | 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
    }

    /// Checks if a byte is a delimiter character according to ISO 32000-1 §7.2.2.
    #[inline]
    pub fn is_delimiter(b: u8) -> bool {
        matches!(
            b,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
    }

    /// Skips all whitespace and comment lines from the current cursor position.
    pub fn skip_whitespace_and_comments(&mut self) {
        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            if Self::is_whitespace(b) {
                self.cursor += 1;
            } else if b == b'%' {
                // Comment extends to the next end-of-line character
                self.cursor += 1;
                while self.cursor < self.data.len() {
                    let cb = self.data[self.cursor];
                    self.cursor += 1;
                    if cb == 0x0A || cb == 0x0D {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    /// Peeks at the next token without advancing the cursor.
    pub fn peek_token(&mut self) -> PdfResult<Option<Token>> {
        let saved_cursor = self.cursor;
        let token = self.next_token();
        self.cursor = saved_cursor;
        token
    }

    /// Scans and returns the next token from the input buffer.
    pub fn next_token(&mut self) -> PdfResult<Option<Token>> {
        self.skip_whitespace_and_comments();

        if self.is_eof() {
            return Ok(None);
        }

        let start_offset = self.cursor;
        let b = self.data[self.cursor];

        // 1. Literal Strings: `(...)`
        if b == b'(' {
            return self.read_literal_string().map(Some);
        }

        // 2. Hex Strings `<...>` or Dictionary Start `<<`
        if b == b'<' {
            if self.cursor + 1 < self.data.len() && self.data[self.cursor + 1] == b'<' {
                self.cursor += 2;
                return Ok(Some(Token::DictionaryStart));
            }
            return self.read_hex_string().map(Some);
        }

        // 3. Dictionary End `>>` or closing delimiter
        if b == b'>' {
            if self.cursor + 1 < self.data.len() && self.data[self.cursor + 1] == b'>' {
                self.cursor += 2;
                return Ok(Some(Token::DictionaryEnd));
            }
            self.cursor += 1;
            return Err(PdfError::LexerError {
                offset: start_offset,
                message: "Unexpected single '>' delimiter".to_string(),
            });
        }

        // 4. Array Delimiters `[` and `]`
        if b == b'[' {
            self.cursor += 1;
            return Ok(Some(Token::ArrayStart));
        }
        if b == b']' {
            self.cursor += 1;
            return Ok(Some(Token::ArrayEnd));
        }

        // 5. Name Objects: `/Name`
        if b == b'/' {
            return self.read_name().map(Some);
        }

        // 6. Regular tokens: Numbers, Keywords, and Booleans
        let token_bytes = self.read_regular_token();
        if token_bytes.is_empty() {
            return Err(PdfError::LexerError {
                offset: start_offset,
                message: format!("Unrecognized character: 0x{:02X}", b),
            });
        }

        self.parse_regular_token(token_bytes, start_offset).map(Some)
    }

    /// Reads literal string according to ISO 32000-1 §7.3.4.2.
    /// Properly handles balanced parentheses and escape sequences.
    fn read_literal_string(&mut self) -> PdfResult<Token> {
        let start_offset = self.cursor;
        self.cursor += 1; // Skip initial '('

        let mut result = Vec::new();
        let mut depth = 1;

        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            self.cursor += 1;

            if b == b'(' {
                depth += 1;
                result.push(b'(');
            } else if b == b')' {
                depth -= 1;
                if depth == 0 {
                    return Ok(Token::LiteralString(PdfString::literal(result)));
                }
                result.push(b')');
            } else if b == b'\\' {
                if self.cursor >= self.data.len() {
                    break;
                }
                let esc = self.data[self.cursor];
                self.cursor += 1;

                match esc {
                    b'n' => result.push(b'\n'),
                    b'r' => result.push(b'\r'),
                    b't' => result.push(b'\t'),
                    b'b' => result.push(0x08),
                    b'f' => result.push(0x0C),
                    b'(' => result.push(b'('),
                    b')' => result.push(b')'),
                    b'\\' => result.push(b'\\'),
                    // Octal escape: \ddd (1 to 3 octal digits)
                    b'0'..=b'7' => {
                        let mut octal_val = (esc - b'0') as u16;
                        let mut count = 1;
                        while count < 3 && self.cursor < self.data.len() {
                            let ob = self.data[self.cursor];
                            if matches!(ob, b'0'..=b'7') {
                                octal_val = (octal_val << 3) + (ob - b'0') as u16;
                                self.cursor += 1;
                                count += 1;
                            } else {
                                break;
                            }
                        }
                        result.push((octal_val & 0xFF) as u8);
                    }
                    // Escaped line break (line continuation)
                    0x0A => { /* ignore line break */ }
                    0x0D => {
                        if self.cursor < self.data.len() && self.data[self.cursor] == 0x0A {
                            self.cursor += 1;
                        }
                    }
                    _ => result.push(esc),
                }
            } else if b == 0x0D {
                // Literal carriage return normalized to newline
                if self.cursor < self.data.len() && self.data[self.cursor] == 0x0A {
                    self.cursor += 1;
                }
                result.push(b'\n');
            } else {
                result.push(b);
            }
        }

        Err(PdfError::UnexpectedEof {
            offset: start_offset,
            context: "Unclosed literal string '('",
        })
    }

    /// Reads hexadecimal string according to ISO 32000-1 §7.3.4.3.
    fn read_hex_string(&mut self) -> PdfResult<Token> {
        let start_offset = self.cursor;
        self.cursor += 1; // Skip initial '<'

        let mut hex_digits = Vec::new();

        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            self.cursor += 1;

            if b == b'>' {
                // If odd number of digits, append final '0' as per §7.3.4.3
                if hex_digits.len() % 2 != 0 {
                    hex_digits.push(b'0');
                }

                let mut bytes = Vec::with_capacity(hex_digits.len() / 2);
                for chunk in hex_digits.chunks_exact(2) {
                    let d1 = Self::hex_char_to_val(chunk[0])?;
                    let d2 = Self::hex_char_to_val(chunk[1])?;
                    bytes.push((d1 << 4) | d2);
                }

                return Ok(Token::HexString(PdfString::hex(bytes)));
            } else if Self::is_whitespace(b) {
                // Ignore whitespace in hex strings
                continue;
            } else if b.is_ascii_hexdigit() {
                hex_digits.push(b);
            } else {
                return Err(PdfError::LexerError {
                    offset: self.cursor - 1,
                    message: format!("Invalid character in hex string: 0x{:02X}", b),
                });
            }
        }

        Err(PdfError::UnexpectedEof {
            offset: start_offset,
            context: "Unclosed hexadecimal string '<'",
        })
    }

    fn hex_char_to_val(b: u8) -> PdfResult<u8> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => Err(PdfError::LexerError {
                offset: 0,
                message: format!("Invalid hex digit 0x{:02X}", b),
            }),
        }
    }

    /// Reads name token and decodes `#XX` hexadecimal escape sequences according to §7.3.5.
    fn read_name(&mut self) -> PdfResult<Token> {
        self.cursor += 1; // Skip leading '/'

        let mut name_bytes = Vec::new();

        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            if Self::is_whitespace(b) || Self::is_delimiter(b) {
                break;
            }

            if b == b'#' {
                if self.cursor + 2 >= self.data.len() {
                    return Err(PdfError::UnexpectedEof {
                        offset: self.cursor,
                        context: "Incomplete hex escape in PDF name",
                    });
                }
                let h1 = self.data[self.cursor + 1];
                let h2 = self.data[self.cursor + 2];
                let val1 = Self::hex_char_to_val(h1)?;
                let val2 = Self::hex_char_to_val(h2)?;
                name_bytes.push((val1 << 4) | val2);
                self.cursor += 3;
            } else {
                name_bytes.push(b);
                self.cursor += 1;
            }
        }

        let name_str = String::from_utf8_lossy(&name_bytes).to_string();
        Ok(Token::Name(PdfName::new(name_str)))
    }

    /// Reads contiguous bytes of a non-delimiter, non-whitespace token.
    fn read_regular_token(&mut self) -> &'a [u8] {
        let start = self.cursor;
        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            if Self::is_whitespace(b) || Self::is_delimiter(b) {
                break;
            }
            self.cursor += 1;
        }
        &self.data[start..self.cursor]
    }

    /// Classifies regular keywords, integers, and real numbers.
    fn parse_regular_token(&self, token_bytes: &'a [u8], offset: usize) -> PdfResult<Token> {
        match token_bytes {
            b"true" => return Ok(Token::Boolean(true)),
            b"false" => return Ok(Token::Boolean(false)),
            b"null" => return Ok(Token::Null),
            b"R" => return Ok(Token::R),
            b"obj" => return Ok(Token::Obj),
            b"endobj" => return Ok(Token::EndObj),
            b"stream" => return Ok(Token::Stream),
            b"endstream" => return Ok(Token::EndStream),
            b"xref" => return Ok(Token::XRef),
            b"trailer" => return Ok(Token::Trailer),
            b"startxref" => return Ok(Token::StartXRef),
            _ => {}
        }

        let token_str = std::str::from_utf8(token_bytes).map_err(|_| PdfError::LexerError {
            offset,
            message: "Invalid UTF-8 in numeric or keyword token".to_string(),
        })?;

        // Try integer parse
        if let Ok(int_val) = token_str.parse::<i64>() {
            return Ok(Token::Integer(int_val));
        }

        // Try real parse
        if let Ok(real_val) = token_str.parse::<f64>() {
            return Ok(Token::Real(real_val));
        }

        Err(PdfError::LexerError {
            offset,
            message: format!("Unknown keyword or malformed number: '{}'", token_str),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lex_primitives() {
        let input = b"true false null 123 -456 3.14159 /Type /Font#20Name [ ] << >> R obj endobj";
        let mut lexer = Lexer::new(input);

        assert_eq!(lexer.next_token().unwrap(), Some(Token::Boolean(true)));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::Boolean(false)));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::Null));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::Integer(123)));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::Integer(-456)));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::Real(3.14159)));
        assert_eq!(
            lexer.next_token().unwrap(),
            Some(Token::Name(PdfName::new("Type")))
        );
        assert_eq!(
            lexer.next_token().unwrap(),
            Some(Token::Name(PdfName::new("Font Name")))
        );
        assert_eq!(lexer.next_token().unwrap(), Some(Token::ArrayStart));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::ArrayEnd));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::DictionaryStart));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::DictionaryEnd));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::R));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::Obj));
        assert_eq!(lexer.next_token().unwrap(), Some(Token::EndObj));
        assert_eq!(lexer.next_token().unwrap(), None);
    }

    #[test]
    fn test_literal_string_balanced_and_escapes() {
        let input = b"(Hello (World) \\n \\(Escaped\\))";
        let mut lexer = Lexer::new(input);

        let token = lexer.next_token().unwrap().unwrap();
        match token {
            Token::LiteralString(s) => {
                assert_eq!(s.bytes, b"Hello (World) \n (Escaped)");
            }
            _ => panic!("Expected LiteralString"),
        }
    }

    #[test]
    fn test_hex_string() {
        let input = b"<48656c6c6f>"; // "Hello"
        let mut lexer = Lexer::new(input);

        let token = lexer.next_token().unwrap().unwrap();
        match token {
            Token::HexString(s) => {
                assert_eq!(s.bytes, b"Hello");
            }
            _ => panic!("Expected HexString"),
        }
    }
}
