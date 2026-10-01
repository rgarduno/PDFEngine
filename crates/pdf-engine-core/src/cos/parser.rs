//! ISO 32000-1 §7.3 Syntactic Parser.
//!
//! Transforms lexical token sequences into strongly-typed `PdfObject` syntax trees.
//! Handles nested arrays, dictionaries, stream objects, and indirect object definitions.

use crate::cos::lexer::{Lexer, Token};
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfStream};
use crate::error::{PdfError, PdfResult};
use crate::security::SecurityLimits;

/// Recursive descent parser for PDF syntactic structures.
pub struct Parser<'a> {
    lexer: Lexer<'a>,
    limits: SecurityLimits,
}

impl<'a> Parser<'a> {
    /// Constructs a new parser over a byte slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            lexer: Lexer::new(data),
            limits: SecurityLimits::default(),
        }
    }

    /// Constructs a parser with custom security limits.
    pub fn with_limits(data: &'a [u8], limits: SecurityLimits) -> Self {
        Self {
            lexer: Lexer::new(data),
            limits,
        }
    }

    /// Creates a parser positioned at a specific byte offset.
    pub fn at_offset(data: &'a [u8], offset: usize) -> Self {
        Self {
            lexer: Lexer::at_offset(data, offset),
            limits: SecurityLimits::default(),
        }
    }

    /// Returns the underlying lexer's byte offset cursor.
    pub fn cursor(&self) -> usize {
        self.lexer.cursor()
    }

    /// Parses the next complete `PdfObject` from the stream.
    pub fn parse_object(&mut self) -> PdfResult<PdfObject> {
        self.parse_object_depth(0)
    }

    /// Internal recursive parser enforcing maximum recursion depth limits.
    fn parse_object_depth(&mut self, depth: usize) -> PdfResult<PdfObject> {
        if depth > self.limits.max_recursion_depth {
            return Err(PdfError::RecursionLimitExceeded {
                id: 0,
                gen: 0,
                max_depth: self.limits.max_recursion_depth,
            });
        }

        let token = match self.lexer.next_token()? {
            Some(t) => t,
            None => {
                return Err(PdfError::UnexpectedEof {
                    offset: self.lexer.cursor(),
                    context: "Expected PDF object token",
                });
            }
        };

        match token {
            Token::Null => Ok(PdfObject::Null),
            Token::Boolean(b) => Ok(PdfObject::Boolean(b)),
            Token::Integer(i) => {
                // Lookahead to check if this is an indirect reference `n g R` or object definition `n g obj`
                let saved_cursor = self.lexer.cursor();
                if let Ok(Some(Token::Integer(gen))) = self.lexer.next_token() {
                    if let Ok(Some(next_tok)) = self.lexer.next_token() {
                        match next_tok {
                            Token::R => {
                                return Ok(PdfObject::Reference(ObjectId::with_generation(
                                    i as u32, gen as u16,
                                )));
                            }
                            _ => {
                                // Not an indirect reference, rewind
                                self.lexer.set_cursor(saved_cursor);
                            }
                        }
                    } else {
                        self.lexer.set_cursor(saved_cursor);
                    }
                } else {
                    self.lexer.set_cursor(saved_cursor);
                }
                Ok(PdfObject::Integer(i))
            }
            Token::Real(r) => Ok(PdfObject::Real(r)),
            Token::Name(name) => Ok(PdfObject::Name(name)),
            Token::LiteralString(s) => Ok(PdfObject::String(s)),
            Token::HexString(s) => Ok(PdfObject::String(s)),
            Token::ArrayStart => self.parse_array(depth + 1),
            Token::DictionaryStart => self.parse_dictionary_or_stream(depth + 1),
            other => Err(PdfError::ParseError {
                offset: self.lexer.cursor(),
                message: format!("Unexpected token when parsing object: {:?}", other),
            }),
        }
    }

    /// Parses an array object `[ item1 item2 ... ]`.
    fn parse_array(&mut self, depth: usize) -> PdfResult<PdfObject> {
        let mut items = Vec::new();

        loop {
            self.lexer.skip_whitespace_and_comments();
            if self.lexer.is_eof() {
                return Err(PdfError::UnexpectedEof {
                    offset: self.lexer.cursor(),
                    context: "Unclosed array delimiter ']'",
                });
            }

            if let Some(Token::ArrayEnd) = self.lexer.peek_token()? {
                let _ = self.lexer.next_token(); // consume ']'
                break;
            }

            items.push(self.parse_object_depth(depth)?);
        }

        Ok(PdfObject::Array(items))
    }

    /// Parses a dictionary `<< /Key Value ... >>` and checks if followed by a `stream` payload.
    fn parse_dictionary_or_stream(&mut self, depth: usize) -> PdfResult<PdfObject> {
        let mut dict = PdfDictionary::new();

        loop {
            self.lexer.skip_whitespace_and_comments();
            if self.lexer.is_eof() {
                return Err(PdfError::UnexpectedEof {
                    offset: self.lexer.cursor(),
                    context: "Unclosed dictionary delimiter '>>'",
                });
            }

            if let Some(Token::DictionaryEnd) = self.lexer.peek_token()? {
                let _ = self.lexer.next_token(); // consume '>>'
                break;
            }

            // Key must be a Name object
            let key_token = self.lexer.next_token()?;
            let key = match key_token {
                Some(Token::Name(n)) => n,
                other => {
                    return Err(PdfError::ParseError {
                        offset: self.lexer.cursor(),
                        message: format!("Expected Name as dictionary key, found: {:?}", other),
                    });
                }
            };

            let val = self.parse_object_depth(depth)?;
            dict.insert(key, val);
        }

        // Check if immediately followed by `stream ... endstream`
        let saved_cursor = self.lexer.cursor();
        self.lexer.skip_whitespace_and_comments();

        if let Ok(Some(Token::Stream)) = self.lexer.next_token() {
            let stream = self.parse_stream_content(dict)?;
            return Ok(PdfObject::Stream(stream));
        }

        self.lexer.set_cursor(saved_cursor);
        Ok(PdfObject::Dictionary(dict))
    }

    /// Extracts stream raw binary content between `stream\n` and `endstream` (ISO 32000-1 §7.3.8.1).
    fn parse_stream_content(&mut self, dict: PdfDictionary) -> PdfResult<PdfStream> {
        // Stream keyword must be followed by CRLF or LF (not just CR)
        let remaining = self.lexer.remaining();
        let mut offset = 0;

        if remaining.starts_with(b"\r\n") {
            offset = 2;
        } else if remaining.starts_with(b"\n") {
            offset = 1;
        } else if remaining.starts_with(b"\r") {
            offset = 1;
        }

        self.lexer.set_cursor(self.lexer.cursor() + offset);

        // If dictionary specifies /Length, read exact bytes; otherwise search for `endstream`
        let length_hint = dict.get("Length").and_then(|l| l.as_i64());

        let content = if let Some(len) = length_hint {
            let len = len as usize;
            let current = self.lexer.cursor();
            let slice = self.lexer.remaining();

            if slice.len() >= len {
                let bytes = slice[..len].to_vec();
                self.lexer.set_cursor(current + len);
                self.lexer.skip_whitespace_and_comments();
                if let Ok(Some(Token::EndStream)) = self.lexer.next_token() {
                    bytes
                } else {
                    // Fallback to scanning for endstream if length was slightly inaccurate
                    self.lexer.set_cursor(current);
                    self.scan_until_endstream()?
                }
            } else {
                self.scan_until_endstream()?
            }
        } else {
            self.scan_until_endstream()?
        };

        Ok(PdfStream::new(dict, content))
    }

    /// Scans bytes until the `endstream` marker is found.
    fn scan_until_endstream(&mut self) -> PdfResult<Vec<u8>> {
        let remaining = self.lexer.remaining();
        let marker = b"endstream";

        if let Some(pos) = remaining.windows(marker.len()).position(|w| w == marker) {
            // Trim trailing newline before endstream if present (CRLF, LF, or CR)
            let mut end_pos = pos;
            if end_pos > 0 && remaining[end_pos - 1] == b'\n' {
                end_pos -= 1;
            }
            if end_pos > 0 && remaining[end_pos - 1] == b'\r' {
                end_pos -= 1;
            }

            let content = remaining[..end_pos].to_vec();
            self.lexer.set_cursor(self.lexer.cursor() + pos + marker.len());
            Ok(content)
        } else {
            Err(PdfError::UnexpectedEof {
                offset: self.lexer.cursor(),
                context: "Missing 'endstream' delimiter",
            })
        }
    }

    /// Parses an indirect object definition `n g obj ... endobj` (ISO 32000-1 §7.3.10).
    pub fn parse_indirect_object(&mut self) -> PdfResult<(ObjectId, PdfObject)> {
        let obj_num = match self.lexer.next_token()? {
            Some(Token::Integer(n)) => n as u32,
            other => {
                return Err(PdfError::ParseError {
                    offset: self.lexer.cursor(),
                    message: format!("Expected object number, found: {:?}", other),
                });
            }
        };

        let gen_num = match self.lexer.next_token()? {
            Some(Token::Integer(g)) => g as u16,
            other => {
                return Err(PdfError::ParseError {
                    offset: self.lexer.cursor(),
                    message: format!("Expected generation number, found: {:?}", other),
                });
            }
        };

        match self.lexer.next_token()? {
            Some(Token::Obj) => {}
            other => {
                return Err(PdfError::ParseError {
                    offset: self.lexer.cursor(),
                    message: format!("Expected 'obj' keyword, found: {:?}", other),
                });
            }
        }

        let obj = self.parse_object()?;

        // Consume trailing `endobj` if present (some streams omit it or have whitespace)
        self.lexer.skip_whitespace_and_comments();
        if let Ok(Some(Token::EndObj)) = self.lexer.peek_token() {
            let _ = self.lexer.next_token();
        }

        Ok((ObjectId::with_generation(obj_num, gen_num), obj))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dictionary_and_arrays() {
        let input = b"<< /Type /Pages /Count 2 /Kids [ 1 0 R 2 0 R ] >>";
        let mut parser = Parser::new(input);
        let obj = parser.parse_object().unwrap();

        let dict = obj.as_dict().unwrap();
        assert_eq!(dict.get("Type").unwrap().as_name().unwrap(), "Pages");
        assert_eq!(dict.get("Count").unwrap().as_i64().unwrap(), 2);

        let kids = dict.get("Kids").unwrap().as_array().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0].as_reference().unwrap(), ObjectId::new(1));
        assert_eq!(kids[1].as_reference().unwrap(), ObjectId::new(2));
    }

    #[test]
    fn test_parse_indirect_object_definition() {
        let input = b"10 0 obj\n<< /Length 13 >>\nstream\nHello, World!\nendstream\nendobj";
        let mut parser = Parser::new(input);
        let (id, obj) = parser.parse_indirect_object().unwrap();

        assert_eq!(id, ObjectId::new(10));
        match obj {
            PdfObject::Stream(s) => {
                assert_eq!(s.content, b"Hello, World!");
            }
            _ => panic!("Expected Stream object"),
        }
    }
}
