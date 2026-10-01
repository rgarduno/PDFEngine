//! ISO 32000-1 §7.5 PDF Document Serializer and Writer.
//!
//! Provides deterministic, byte-accurate serialization for PDF objects,
//! cross-reference tables, trailers, and incremental updates.

use std::io::{self, Write};

use crate::cos::object::{ObjectId, PdfDictionary, PdfObject, PdfString, StringFormat};

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
