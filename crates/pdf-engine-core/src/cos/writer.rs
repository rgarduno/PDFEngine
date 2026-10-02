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

        // Group contiguous object numbers into subsections
        let mut sorted = offsets.to_vec();
        sorted.sort_by_key(|&(id, _)| id.number);

        self.write_all(b"xref\n")?;

        if sorted.is_empty() {
            self.write_all(b"0 1\n0000000000 65535 f \n")?;
        } else {
            // First entry: object 0 (free head)
            let first_num = sorted[0].0.number;
            if first_num == 1 {
                self.write_all(format!("0 {}\n", sorted.len() + 1).as_bytes())?;
                self.write_all(b"0000000000 65535 f \n")?;
                for (id, offset) in &sorted {
                    self.write_all(
                        format!("{:010} {:05} n \n", offset, id.generation).as_bytes(),
                    )?;
                }
            } else {
                self.write_all(format!("{} {}\n", first_num, sorted.len()).as_bytes())?;
                for (id, offset) in &sorted {
                    self.write_all(
                        format!("{:010} {:05} n \n", offset, id.generation).as_bytes(),
                    )?;
                }
            }
        }

        // Write trailer
        self.write_all(b"trailer\n")?;
        self.write_dictionary(trailer_dict)?;
        self.write_all(format!("\nstartxref\n{}\n%%EOF\n", startxref_offset).as_bytes())?;

        Ok(())
    }

    /// Writes a modern compressed cross-reference stream (/XRef) according to ISO 32000-1 §7.5.8.
    ///
    /// Packs cross-reference entries (both uncompressed type 1, compressed in /ObjStm type 2,
    /// and free slots type 0) into a binary table, compresses it with Flate, embeds the trailer
    /// dictionary entries into the stream dictionary, and writes `startxref` pointing to the
    /// XRef stream object.
    pub fn write_xref_stream(
        &mut self,
        xref_entries: &BTreeMap<ObjectId, XRefEntry>,
        trailer_dict: &PdfDictionary,
    ) -> io::Result<usize> {
        let max_id = xref_entries.keys().map(|id| id.number).max().unwrap_or(0);
        let xref_stream_id = ObjectId::new(max_id + 1);
        let startxref_offset = self.bytes_written;

        // ISO 32000-1 §7.5.8: /W [ 1 4 2 ]
        // Field 1: 1 byte (type: 0 = Free, 1 = InUse, 2 = Compressed in ObjStm)
        // Field 2: 4 bytes (offset in file, or container ObjStm object number)
        // Field 3: 2 bytes (generation number, or index inside ObjStm)
        let w1: usize = 1;
        let w2: usize = 4;
        let w3: usize = 2;
        let total_objects = (xref_stream_id.number + 1) as usize;
        let entry_size = w1 + w2 + w3;

        let mut raw_table = Vec::with_capacity(total_objects * entry_size);

        for obj_num in 0..=xref_stream_id.number {
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
                        // Unused slot treated as free
                        raw_table.push(0);
                        raw_table.extend_from_slice(&0u32.to_be_bytes());
                        raw_table.extend_from_slice(&65535u16.to_be_bytes());
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
        stream_dict.insert("Size", (xref_stream_id.number + 1) as i64);

        let mut w_array = PdfArray::new();
        w_array.push(PdfObject::Integer(w1 as i64));
        w_array.push(PdfObject::Integer(w2 as i64));
        w_array.push(PdfObject::Integer(w3 as i64));
        stream_dict.insert("W", PdfObject::Array(w_array));

        let mut index_array = PdfArray::new();
        index_array.push(PdfObject::Integer(0i64));
        index_array.push(PdfObject::Integer((xref_stream_id.number + 1) as i64));
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

    #[test]
    fn test_write_primitives() {
        let mut buffer = Vec::new();
        let mut writer = Writer::new(&mut buffer);

        let dict = PdfDictionary::new();
        writer.write_object(&PdfObject::Dictionary(dict)).unwrap();

        assert_eq!(String::from_utf8(buffer).unwrap(), "<<\n>>");
    }
}
