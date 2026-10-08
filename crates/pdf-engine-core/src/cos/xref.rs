//! ISO 32000-1 §7.5.4 & §7.5.8 Cross-Reference Tables and Streams.
//!
//! Handles both traditional ASCII `xref` tables and modern compressed `XRef Streams` (PDF 1.5+).
//! Resolves incremental updates through `/Prev` chains and locates objects stored in
//! `Object Streams` (`/ObjStm`).

use std::collections::BTreeMap;

use crate::cos::filters::decode_stream;
use crate::cos::lexer::{Lexer, Token};
use crate::cos::object::{ObjectId, PdfDictionary, PdfObject};
use crate::cos::parser::Parser;
use crate::error::{PdfError, PdfResult};
use crate::security::SecurityLimits;

/// An entry within a PDF cross-reference table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XRefEntry {
    /// Normal uncompressed object located at an absolute byte offset in the PDF file.
    InUse { offset: u64, generation: u16 },
    /// Object stored compressed within an Object Stream (`/ObjStm`) (ISO 32000-1 §7.5.7).
    Compressed {
        container_id: u32,
        index_in_stream: u16,
    },
    /// Free object slot in the linked list of deleted objects.
    Free {
        next_free_object: u32,
        generation: u16,
    },
}

/// Aggregated cross-reference index mapping `ObjectId` to its physical or stream location.
#[derive(Debug, Clone, Default)]
pub struct XRefTable {
    /// Mapping of all known objects to their location entries.
    pub entries: BTreeMap<ObjectId, XRefEntry>,
    /// The trailer dictionary associated with the document.
    pub trailer: PdfDictionary,
}

impl XRefTable {
    /// Creates an empty cross-reference table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates an entry in the table.
    pub fn insert(&mut self, id: ObjectId, entry: XRefEntry) {
        self.entries.insert(id, entry);
    }

    /// Looks up an object's location entry by its identifier.
    pub fn get(&self, id: ObjectId) -> Option<&XRefEntry> {
        self.entries.get(&id)
    }

    /// Scans backwards from the end of the file buffer to find the `startxref` keyword.
    pub fn find_startxref(data: &[u8]) -> PdfResult<usize> {
        let scan_limit = 2048.min(data.len());
        let tail = &data[data.len() - scan_limit..];
        let marker = b"startxref";

        if let Some(pos) = tail.windows(marker.len()).rposition(|w| w == marker) {
            let offset_start = data.len() - scan_limit + pos + marker.len();
            let mut lexer = Lexer::at_offset(data, offset_start);
            match lexer.next_token()? {
                Some(Token::Integer(offset)) => Ok(offset as usize),
                other => Err(PdfError::InvalidXRef {
                    offset: offset_start,
                    message: format!(
                        "Expected integer offset after startxref, found: {:?}",
                        other
                    ),
                }),
            }
        } else {
            Err(PdfError::InvalidXRef {
                offset: data.len(),
                message: "Missing 'startxref' keyword near end of PDF file".to_string(),
            })
        }
    }

    /// Loads the complete cross-reference chain starting from `startxref` offset.
    pub fn load_chain(
        data: &[u8],
        start_offset: usize,
        limits: &SecurityLimits,
    ) -> PdfResult<Self> {
        let mut table = XRefTable::new();
        let mut current_offset = Some(start_offset);
        let mut visited_offsets = std::collections::HashSet::new();

        while let Some(offset) = current_offset {
            if !visited_offsets.insert(offset) {
                return Err(PdfError::SecurityLimitExceeded(format!(
                    "Circular reference loop detected in /Prev XRef chain at offset {}",
                    offset
                )));
            }

            if offset >= data.len() {
                return Err(PdfError::InvalidXRef {
                    offset,
                    message: "XRef offset points beyond end of file buffer".to_string(),
                });
            }

            let mut lexer = Lexer::at_offset(data, offset);
            lexer.skip_whitespace_and_comments();

            match lexer.peek_token()? {
                Some(Token::XRef) => {
                    // Classic ASCII cross-reference table
                    let _ = lexer.next_token(); // consume 'xref'
                    Self::parse_classic_xref(&mut lexer, &mut table, limits)?;

                    // Parse trailer dictionary
                    lexer.skip_whitespace_and_comments();
                    match lexer.next_token()? {
                        Some(Token::Trailer) => {
                            let mut parser = Parser::at_offset(data, lexer.cursor());
                            let trailer_obj = parser.parse_object()?;
                            if let PdfObject::Dictionary(dict) = trailer_obj {
                                // Merge entries from earlier trailer if not yet set
                                for (k, v) in dict.0 {
                                    table.trailer.0.entry(k).or_insert(v);
                                }

                                // Check for /Prev incremental update pointer
                                current_offset = table
                                    .trailer
                                    .get("Prev")
                                    .and_then(|o| o.as_i64())
                                    .map(|o| o as usize);
                            } else {
                                return Err(PdfError::InvalidXRef {
                                    offset: lexer.cursor(),
                                    message: "Trailer must be a dictionary".to_string(),
                                });
                            }
                        }
                        other => {
                            return Err(PdfError::InvalidXRef {
                                offset: lexer.cursor(),
                                message: format!("Expected 'trailer' keyword, found: {:?}", other),
                            });
                        }
                    }
                }
                Some(Token::Integer(_)) => {
                    // Modern compressed XRef Stream (PDF 1.5+)
                    let mut parser = Parser::at_offset(data, offset);
                    let (id, obj) = parser.parse_indirect_object()?;

                    if let PdfObject::Stream(stream) = obj {
                        Self::parse_xref_stream(&stream, &mut table, limits)?;

                        // Merge trailer fields from XRef stream dictionary
                        for (k, v) in stream.dict.0 {
                            table.trailer.0.entry(k).or_insert(v);
                        }

                        current_offset = table
                            .trailer
                            .get("Prev")
                            .and_then(|o| o.as_i64())
                            .map(|o| o as usize);
                    } else {
                        return Err(PdfError::InvalidXRef {
                            offset,
                            message: format!("Expected XRef stream at object {}", id),
                        });
                    }
                }
                other => {
                    return Err(PdfError::InvalidXRef {
                        offset,
                        message: format!("Unrecognized XRef header token: {:?}", other),
                    });
                }
            }
        }

        Ok(table)
    }

    /// Parses classic ASCII xref table subsections according to ISO 32000-1 §7.5.4.
    fn parse_classic_xref(
        lexer: &mut Lexer,
        table: &mut XRefTable,
        limits: &SecurityLimits,
    ) -> PdfResult<()> {
        loop {
            lexer.skip_whitespace_and_comments();
            let first_tok = lexer.peek_token()?;

            let first_id = match first_tok {
                Some(Token::Integer(id)) => id as u32,
                Some(Token::Trailer) => break, // Reached trailer keyword
                other => {
                    return Err(PdfError::InvalidXRef {
                        offset: lexer.cursor(),
                        message: format!(
                            "Expected subsection start or 'trailer', found: {:?}",
                            other
                        ),
                    });
                }
            };
            let _ = lexer.next_token(); // consume first_id

            let count_i = match lexer.next_token()? {
                Some(Token::Integer(c)) => c,
                other => {
                    return Err(PdfError::InvalidXRef {
                        offset: lexer.cursor(),
                        message: format!("Expected subsection count integer, found: {:?}", other),
                    });
                }
            };
            if count_i < 0 {
                return Err(PdfError::InvalidXRef {
                    offset: lexer.cursor(),
                    message: "XRef subsection count cannot be negative".to_string(),
                });
            }
            let added = count_i as usize;
            limits.validate_object_count(table.entries.len().saturating_add(added))?;
            let count = count_i as u32;

            for i in 0..count {
                let number = first_id
                    .checked_add(i)
                    .ok_or_else(|| PdfError::InvalidXRef {
                        offset: lexer.cursor(),
                        message: "XRef subsection object number overflow".to_string(),
                    })?;
                let current_id = ObjectId::new(number);

                let offset_val = match lexer.next_token()? {
                    Some(Token::Integer(o)) => o as u64,
                    other => {
                        return Err(PdfError::InvalidXRef {
                            offset: lexer.cursor(),
                            message: format!("Expected offset in xref entry, found: {:?}", other),
                        });
                    }
                };

                let gen_val = match lexer.next_token()? {
                    Some(Token::Integer(g)) => g as u16,
                    other => {
                        return Err(PdfError::InvalidXRef {
                            offset: lexer.cursor(),
                            message: format!(
                                "Expected generation in xref entry, found: {:?}",
                                other
                            ),
                        });
                    }
                };

                // Entry status flag 'n' (in-use) or 'f' (free)
                lexer.skip_whitespace_and_comments();
                let remaining = lexer.remaining();
                if remaining.is_empty() {
                    return Err(PdfError::UnexpectedEof {
                        offset: lexer.cursor(),
                        context: "Truncated xref entry",
                    });
                }

                let flag = remaining[0];
                lexer.set_cursor(lexer.cursor() + 1);

                match flag {
                    b'n' => {
                        table.entries.entry(current_id).or_insert(XRefEntry::InUse {
                            offset: offset_val,
                            generation: gen_val,
                        });
                    }
                    b'f' => {
                        table.entries.entry(current_id).or_insert(XRefEntry::Free {
                            next_free_object: offset_val as u32,
                            generation: gen_val,
                        });
                    }
                    _ => {
                        return Err(PdfError::InvalidXRef {
                            offset: lexer.cursor() - 1,
                            message: format!("Invalid xref entry flag: '{}'", flag as char),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Decodes and indexes a compressed XRef Stream according to ISO 32000-1 §7.5.8.
    fn parse_xref_stream(
        stream: &crate::cos::object::PdfStream,
        table: &mut XRefTable,
        limits: &SecurityLimits,
    ) -> PdfResult<()> {
        let filter_name = stream
            .dict
            .get("Filter")
            .and_then(|f| f.as_name())
            .unwrap_or("FlateDecode");

        let decode_parms = stream.dict.get("DecodeParms").and_then(|p| p.as_dict());
        let decompressed = decode_stream(filter_name, decode_parms, &stream.content, limits)?;

        // Read /W field array: widths of the 3 fields
        let w_array = stream
            .dict
            .get("W")
            .and_then(|w| w.as_array())
            .ok_or_else(|| PdfError::InvalidXRef {
                offset: 0,
                message: "XRef stream missing mandatory /W array".to_string(),
            })?;

        if w_array.len() < 3 {
            return Err(PdfError::InvalidXRef {
                offset: 0,
                message: "XRef stream /W array must contain at least 3 elements".to_string(),
            });
        }

        let w1 = w_array[0].as_i64().unwrap_or(0) as usize;
        let w2 = w_array[1].as_i64().unwrap_or(0) as usize;
        let w3 = w_array[2].as_i64().unwrap_or(0) as usize;
        let entry_size = w1 + w2 + w3;

        if entry_size == 0 {
            return Err(PdfError::InvalidXRef {
                offset: 0,
                message: "Total XRef stream entry size cannot be 0".to_string(),
            });
        }

        // Read /Index array: pairs of [first_object count ...]
        let index_pairs: Vec<(u32, u32)> =
            if let Some(idx_arr) = stream.dict.get("Index").and_then(|i| i.as_array()) {
                let mut pairs = Vec::new();
                for pair in idx_arr.chunks_exact(2) {
                    let first = pair[0].as_i64().unwrap_or(0);
                    let count = pair[1].as_i64().unwrap_or(0);
                    if first < 0 || count < 0 {
                        return Err(PdfError::InvalidXRef {
                            offset: 0,
                            message: "XRef stream /Index values cannot be negative".to_string(),
                        });
                    }
                    limits.validate_object_count(count as usize)?;
                    pairs.push((first as u32, count as u32));
                }
                pairs
            } else {
                // Default: single subsection [0 /Size]
                let size = stream
                    .dict
                    .get("Size")
                    .and_then(|s| s.as_i64())
                    .unwrap_or(0);
                if size < 0 {
                    return Err(PdfError::InvalidXRef {
                        offset: 0,
                        message: "XRef stream /Size cannot be negative".to_string(),
                    });
                }
                limits.validate_object_count(size as usize)?;
                vec![(0, size as u32)]
            };

        let mut byte_offset = 0;
        for (first_id, count) in index_pairs {
            for i in 0..count {
                if byte_offset + entry_size > decompressed.len() {
                    break;
                }

                let number = match first_id.checked_add(i) {
                    Some(number) => number,
                    None => {
                        return Err(PdfError::InvalidXRef {
                            offset: 0,
                            message: "XRef stream object number overflow".to_string(),
                        });
                    }
                };
                let current_id = ObjectId::new(number);
                if !table.entries.contains_key(&current_id) {
                    limits.validate_object_count(table.entries.len() + 1)?;
                }
                let slice = &decompressed[byte_offset..byte_offset + entry_size];
                byte_offset += entry_size;

                let field1 = read_int_bytes(&slice[..w1], w1.min(1));
                let field2 = read_int_bytes(&slice[w1..w1 + w2], 0);
                let field3 = read_int_bytes(&slice[w1 + w2..], 0);

                let entry = match field1 {
                    0 => XRefEntry::Free {
                        next_free_object: field2 as u32,
                        generation: field3 as u16,
                    },
                    1 => XRefEntry::InUse {
                        offset: field2,
                        generation: field3 as u16,
                    },
                    2 => XRefEntry::Compressed {
                        container_id: field2 as u32,
                        index_in_stream: field3 as u16,
                    },
                    _ => continue,
                };

                table.entries.entry(current_id).or_insert(entry);
            }
        }

        Ok(())
    }
}

/// Helper function to parse big-endian integer from variable byte width.
fn read_int_bytes(bytes: &[u8], default_val: usize) -> u64 {
    if bytes.is_empty() {
        return default_val as u64;
    }
    let mut val: u64 = 0;
    for &b in bytes {
        val = (val << 8) | (b as u64);
    }
    val
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::SecurityLimits;

    #[test]
    fn test_parse_classic_xref_subsection() {
        let input = b"xref\n0 2\n0000000000 65535 f \n0000000017 00000 n \ntrailer\n<< /Size 2 >>";
        let mut table = XRefTable::new();
        let mut lexer = Lexer::new(input);
        let _ = lexer.next_token(); // consume xref

        XRefTable::parse_classic_xref(&mut lexer, &mut table, &SecurityLimits::default()).unwrap();

        assert_eq!(
            table.get(ObjectId::new(0)),
            Some(&XRefEntry::Free {
                next_free_object: 0,
                generation: 65535
            })
        );
        assert_eq!(
            table.get(ObjectId::new(1)),
            Some(&XRefEntry::InUse {
                offset: 17,
                generation: 0
            })
        );
    }

    #[test]
    fn negative_xref_count_is_rejected() {
        let input = b"xref\n0 -1\ntrailer\n<< /Size 1 >>";
        let mut table = XRefTable::new();
        let mut lexer = Lexer::new(input);
        let _ = lexer.next_token();
        let error =
            XRefTable::parse_classic_xref(&mut lexer, &mut table, &SecurityLimits::default())
                .unwrap_err();
        assert!(error.to_string().contains("cannot be negative"));
        assert!(table.entries.is_empty());
    }

    #[test]
    fn xref_subsection_stops_at_the_object_cap() {
        let input = b"xref\n0 5\n0000000000 65535 f \n0000000017 00000 n \n0000000034 00000 n \n0000000051 00000 n \n0000000068 00000 n \ntrailer\n<< /Size 5 >>";
        let mut table = XRefTable::new();
        let mut lexer = Lexer::new(input);
        let _ = lexer.next_token();
        let mut limits = SecurityLimits::default();
        limits.max_object_count = 2;
        let error = XRefTable::parse_classic_xref(&mut lexer, &mut table, &limits).unwrap_err();
        assert!(error.to_string().contains("Object count"));
    }
}
