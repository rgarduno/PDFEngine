//! ISO 32000-1 §7.5 PDF Document Serializer and Writer.
//!
//! Provides deterministic, byte-accurate serialization for PDF objects,
//! cross-reference tables, trailers, and incremental updates.

use std::collections::BTreeMap;
use std::io::{self, Write};

use crate::cos::filters::{encode_flate, Compression};
use crate::cos::object::{
    ObjectId, PdfArray, PdfDictionary, PdfName, PdfObject, PdfString, StringFormat,
};
use crate::cos::xref::XRefEntry;

/// Serializes PDF objects and document structures into byte streams.
pub struct Writer<W: Write> {
    writer: W,
    bytes_written: usize,
}

impl<W: Write> Writer<W> {
    /// Creates a new writer wrapping an underlying `Write` sink.
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            bytes_written: 0,
        }
    }

    /// Returns the total number of bytes written to the sink.
    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    /// Writes raw bytes and tracks total byte count.
    pub fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.writer.write_all(buf)?;
        self.bytes_written += buf.len();
        Ok(())
    }

    /// Writes a standard PDF header (e.g. `%PDF-1.7` followed by binary comment).
    pub fn write_header(&mut self, version: &str) -> io::Result<()> {
        self.write_all(format!("%PDF-{}\n", version).as_bytes())?;
        // 4 binary bytes (> 127) as required by ISO 32000-1 §7.5.2 to signal binary content
        self.write_all(b"%\xE2\xE3\xCF\xD3\n")?;
        Ok(())
    }

    /// Serializes an indirect object definition `n g obj ... endobj`.
    pub fn write_indirect_object(&mut self, id: ObjectId, obj: &PdfObject) -> io::Result<usize> {
        let offset = self.bytes_written;
        self.write_all(format!("{} {} obj\n", id.number, id.generation).as_bytes())?;
        self.write_object(obj)?;
        self.write_all(b"\nendobj\n")?;
        Ok(offset)
    }

    /// Recursively serializes any `PdfObject`.
    pub fn write_object(&mut self, obj: &PdfObject) -> io::Result<()> {
        match obj {
            PdfObject::Null => self.write_all(b"null"),
            PdfObject::Boolean(b) => {
                if *b {
                    self.write_all(b"true")
                } else {
                    self.write_all(b"false")
                }
            }
            PdfObject::Integer(i) => self.write_all(i.to_string().as_bytes()),
            PdfObject::Real(r) => {
                // Format real number without unnecessary scientific notation
                if r.is_finite() {
                    self.write_all(format!("{:.5}", r).trim_end_matches('0').trim_end_matches('.').as_bytes())
                } else {
                    self.write_all(b"0")
                }
            }
            PdfObject::Name(name) => {
                self.write_all(b"/")?;
                for &b in name.as_str().as_bytes() {
                    if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                        self.write_all(&[b])?;
                    } else {
                        self.write_all(format!("#{:02X}", b).as_bytes())?;
                    }
                }
                Ok(())
            }
            PdfObject::String(s) => self.write_string(s),
            PdfObject::Array(arr) => {
                self.write_all(b"[ ")?;
                for item in arr {
                    self.write_object(item)?;
                    self.write_all(b" ")?;
                }
                self.write_all(b"]")
            }
            PdfObject::Dictionary(dict) => self.write_dictionary(dict),
            PdfObject::Stream(stream) => {
                self.write_dictionary(&stream.dict)?;
                self.write_all(b"\nstream\n")?;
                self.write_all(&stream.content)?;
                self.write_all(b"\nendstream")
            }
            PdfObject::Reference(id) => {
                self.write_all(format!("{} {} R", id.number, id.generation).as_bytes())
            }
        }
    }

    /// Serializes a string literal or hexadecimal sequence.
    fn write_string(&mut self, s: &PdfString) -> io::Result<()> {
        match s.format {
            StringFormat::Hexadecimal => {
                self.write_all(b"<")?;
                for &b in &s.bytes {
                    self.write_all(format!("{:02X}", b).as_bytes())?;
                }
                self.write_all(b">")
            }
            StringFormat::Literal => {
                self.write_all(b"(")?;
                for &b in &s.bytes {
                    match b {
                        b'\n' => self.write_all(b"\\n")?,
                        b'\r' => self.write_all(b"\\r")?,
                        b'\t' => self.write_all(b"\\t")?,
                        b'(' => self.write_all(b"\\(")?,
                        b')' => self.write_all(b"\\)")?,
                        b'\\' => self.write_all(b"\\\\")?,
                        _ => self.write_all(&[b])?,
                    }
                }
                self.write_all(b")")
            }
        }
    }

    /// Serializes a dictionary `<< /Key Val ... >>`.
    pub fn write_dictionary(&mut self, dict: &PdfDictionary) -> io::Result<()> {
        self.write_all(b"<<\n")?;
        for (key, val) in &dict.0 {
            self.write_all(format!("  /{} ", key.as_str()).as_bytes())?;
            self.write_object(val)?;
            self.write_all(b"\n")?;
        }
        self.write_all(b">>")
    }

    /// Writes a complete classic cross-reference table and trailer block.
    pub fn write_xref_and_trailer(
        &mut self,
        offsets: &[(ObjectId, usize)],
        trailer_dict: &PdfDictionary,
    ) -> io::Result<()> {
        let startxref_offset = self.bytes_written;

        // Object 0 is the free-list head. Later numbers are written in order,
        // and a hole starts a new subsection. One subsection of `0 N` is only
        // valid when the in-use numbers are exactly 1, 2, ..., N-1.
        let mut sorted = offsets.to_vec();
        sorted.sort_by_key(|&(id, _)| (id.number, id.generation));
        sorted.retain(|(id, _)| id.number != 0);

        self.write_all(b"xref\n")?;

        let mut entries: Vec<(u32, u16, usize, bool)> = Vec::with_capacity(sorted.len() + 1);
        entries.push((0, 65535, 0, false));
        for (id, offset) in &sorted {
            entries.push((id.number, id.generation, *offset, true));
        }

        let mut index = 0;
        while index < entries.len() {
            let start = entries[index].0;
            let mut end = index + 1;
            while end < entries.len() && entries[end].0 == entries[end - 1].0.saturating_add(1) {
                end += 1;
            }
            self.write_all(format!("{} {}\n", start, end - index).as_bytes())?;
            for &(_, generation, offset, in_use) in &entries[index..end] {
                if in_use {
                    self.write_all(format!("{:010} {:05} n \n", offset, generation).as_bytes())?;
                } else {
                    self.write_all(b"0000000000 65535 f \n")?;
                }
            }
            index = end;
        }

        // Write trailer
        self.write_all(b"trailer\n")?;
        self.write_dictionary(trailer_dict)?;
        self.write_all(format!("\nstartxref\n{}\n%%EOF\n", startxref_offset).as_bytes())?;

        Ok(())
    }

    /// Writes a modern compressed cross-reference stream (/XRef) according to ISO 32000-1 §7.5.8.
    ///
    /// Packs occupied generation-0 entries (uncompressed type 1, compressed in /ObjStm type 2,
    /// and explicit free slots type 0) into a binary table. Unoccupied object numbers are left
    /// out of `/Index`, so the table grows with the entry count rather than with the highest
    /// object number. The table is then Flate-compressed. Trailer entries are copied into the
    /// stream dictionary, and `startxref` points at the XRef stream object.
    pub fn write_xref_stream(
        &mut self,
        xref_entries: &BTreeMap<ObjectId, XRefEntry>,
        trailer_dict: &PdfDictionary,
    ) -> io::Result<usize> {
        let max_existing = xref_entries.keys().map(|id| id.number).max().unwrap_or(0);
        let xref_number = max_existing.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "cross-reference object number overflow",
            )
        })?;
        let xref_stream_id = ObjectId::new(xref_number);
        let startxref_offset = self.bytes_written;

        // ISO 32000-1 §7.5.8: /W [ 1 4 2 ]
        // Field 1: 1 byte (type: 0 = Free, 1 = InUse, 2 = Compressed in ObjStm)
        // Field 2: 4 bytes (offset in file, or container ObjStm object number)
        // Field 3: 2 bytes (generation number, or index inside ObjStm)
        let w1: usize = 1;
        let w2: usize = 4;
        let w3: usize = 2;
        let entry_size = w1 + w2 + w3;

        // Object 0 is the free-list head. Every other row is a generation-0 entry
        // that actually exists, plus the cross-reference stream object itself.
        let mut numbers: Vec<u32> = xref_entries
            .keys()
            .filter(|id| id.generation == 0 && id.number != 0)
            .map(|id| id.number)
            .collect();
        numbers.push(0);
        numbers.push(xref_number);
        numbers.sort_unstable();
        numbers.dedup();

        let mut subsections: Vec<(u32, u32)> = Vec::new();
        let mut index = 0;
        while index < numbers.len() {
            let first = numbers[index];
            let mut last = first;
            index += 1;
            while index < numbers.len()
                && last < u32::MAX
                && numbers[index] == last + 1
            {
                last = numbers[index];
                index += 1;
            }
            let count = last.checked_sub(first).and_then(|span| span.checked_add(1)).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cross-reference object number overflow",
                )
            })?;
            subsections.push((first, count));
        }

        let row_count: usize = subsections.iter().map(|(_, count)| *count as usize).sum();
        let mut raw_table = Vec::with_capacity(row_count * entry_size);

        for &(first, count) in &subsections {
            for offset in 0..count {
                let obj_num = first + offset;
                if obj_num == 0 {
                    // Object 0: free entry head (next free 0, gen 65535)
                    raw_table.push(0);
                    raw_table.extend_from_slice(&0u32.to_be_bytes());
                    raw_table.extend_from_slice(&65535u16.to_be_bytes());
                } else if obj_num == xref_stream_id.number {
                    // The XRef stream object itself: Type 1 at startxref_offset, gen 0
                    raw_table.push(1);
                    raw_table.extend_from_slice(&(startxref_offset as u32).to_be_bytes());
                    raw_table.extend_from_slice(&0u16.to_be_bytes());
                } else {
                    let id = ObjectId::new(obj_num);
                    match xref_entries.get(&id) {
                        Some(XRefEntry::InUse { offset, generation }) => {
                            raw_table.push(1);
                            raw_table.extend_from_slice(&(*offset as u32).to_be_bytes());
                            raw_table.extend_from_slice(&(*generation).to_be_bytes());
                        }
                        Some(XRefEntry::Compressed {
                            container_id,
                            index_in_stream,
                        }) => {
                            raw_table.push(2);
                            raw_table.extend_from_slice(&(*container_id).to_be_bytes());
                            raw_table.extend_from_slice(&(*index_in_stream).to_be_bytes());
                        }
                        Some(XRefEntry::Free {
                            next_free_object,
                            generation,
                        }) => {
                            raw_table.push(0);
                            raw_table.extend_from_slice(&(*next_free_object).to_be_bytes());
                            raw_table.extend_from_slice(&(*generation).to_be_bytes());
                        }
                        None => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "cross-reference subsection listed an unoccupied object",
                            ));
                        }
                    }
                }
            }
        }

        // Compress the binary table using FlateDecode
        let compressed_table = encode_flate(&raw_table, Compression::best())
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        // Construct XRef stream dictionary
        let mut stream_dict = trailer_dict.clone();
        stream_dict.insert("Type", PdfName::new("XRef"));
        // `/Size` is one past the highest object number. Adding in i64 avoids a
        // wrapping `u32` when that number is the top of the range.
        let size = (xref_stream_id.number as i64) + 1;
        stream_dict.insert("Size", size);

        let mut w_array = PdfArray::new();
        w_array.push(PdfObject::Integer(w1 as i64));
        w_array.push(PdfObject::Integer(w2 as i64));
        w_array.push(PdfObject::Integer(w3 as i64));
        stream_dict.insert("W", PdfObject::Array(w_array));

        let mut index_array = PdfArray::new();
        for &(first, count) in &subsections {
            index_array.push(PdfObject::Integer(first as i64));
            index_array.push(PdfObject::Integer(count as i64));
        }
        stream_dict.insert("Index", PdfObject::Array(index_array));

        stream_dict.insert("Filter", PdfName::new("FlateDecode"));
        stream_dict.insert("Length", compressed_table.len() as i64);
        stream_dict.0.remove(&PdfName::new("Prev"));

        // Write indirect object header
        self.write_all(format!("{} 0 obj\n", xref_stream_id.number).as_bytes())?;
        self.write_dictionary(&stream_dict)?;
        self.write_all(b"\nstream\n")?;
        self.write_all(&compressed_table)?;
        self.write_all(b"\nendstream\nendobj\n")?;

        // Write startxref and EOF
        self.write_all(format!("startxref\n{}\n%%EOF\n", startxref_offset).as_bytes())?;

        Ok(startxref_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::PdfDocument;

    #[test]
    fn test_write_primitives() {
        let mut buffer = Vec::new();
        let mut writer = Writer::new(&mut buffer);

        let dict = PdfDictionary::new();
        writer.write_object(&PdfObject::Dictionary(dict)).unwrap();

        assert_eq!(String::from_utf8(buffer).unwrap(), "<<\n>>");
    }

    #[test]
    fn sparse_xref_stream_omits_unoccupied_numbers() {
        let mut buffer = Vec::new();
        let mut writer = Writer::new(&mut buffer);
        writer.write_header("1.7").unwrap();

        let low_id = ObjectId::new(1);
        let high_id = ObjectId::new(1_000_000);
        let low_offset = writer
            .write_indirect_object(low_id, &PdfObject::Integer(11))
            .unwrap();
        let high_offset = writer
            .write_indirect_object(high_id, &PdfObject::Integer(22))
            .unwrap();

        let mut entries = BTreeMap::new();
        entries.insert(
            low_id,
            XRefEntry::InUse {
                offset: low_offset as u64,
                generation: 0,
            },
        );
        entries.insert(
            high_id,
            XRefEntry::InUse {
                offset: high_offset as u64,
                generation: 0,
            },
        );
        writer
            .write_xref_stream(&entries, &PdfDictionary::new())
            .unwrap();

        assert!(buffer.len() < 50_000, "sparse xref grew to {} bytes", buffer.len());
        let text = String::from_utf8_lossy(&buffer);
        assert!(text.contains("[ 0 2 1000000 2 ]"));
        assert!(!text.contains("[ 0 1000002 ]"));

        let mut loaded = PdfDocument::load(&buffer).unwrap();
        assert_eq!(loaded.get_object(low_id).unwrap(), PdfObject::Integer(11));
        assert_eq!(loaded.get_object(high_id).unwrap(), PdfObject::Integer(22));
        assert!(loaded.get_object(ObjectId::new(2)).is_err());
    }
}
