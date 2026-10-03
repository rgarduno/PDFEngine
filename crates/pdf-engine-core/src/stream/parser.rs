//! Content Stream Lexer, Parser, and AST Serializer according to ISO 32000-1 §7.8.
//!
//! Transforms raw content stream byte slices into lossless hierarchical `ContentAst` trees
//! and serializes them back to exact PDF operator bytes.

use crate::cos::lexer::{Lexer, Token};
use crate::cos::object::{PdfName, PdfObject, PdfString};
use crate::error::{PdfError, PdfResult};
use crate::stream::ast::{ContentAst, ContentNode, Operation};

/// Parser converting raw content stream bytes into a `ContentAst`.
pub struct ContentParser<'a> {
    data: &'a [u8],
}

impl<'a> ContentParser<'a> {
    /// Creates a new content parser over a page stream byte slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Parses the entire content stream into a `ContentAst`.
    pub fn parse(&self) -> PdfResult<ContentAst> {
        let mut tokenizer = ContentStreamTokenizer::new(self.data);
        let ops = tokenizer.tokenize_all()?;
        Ok(build_ast_from_operations(ops))
    }
}

/// Nested arrays and dictionaries stop at this depth.
/// A deeper stream is rejected instead of growing the call stack.
const MAX_CONTENT_NESTING: usize = 32;

/// Tokenizer specialized for postfix operator identification in content streams.
pub struct ContentStreamTokenizer<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> ContentStreamTokenizer<'a> {
    /// Creates a tokenizer for content streams.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }

    /// Parses content stream into an ordered sequence of `Operation` instructions.
    pub fn tokenize_all(&mut self) -> PdfResult<Vec<Operation>> {
        let mut operations = Vec::new();
        let mut operands = Vec::new();

        while self.cursor < self.data.len() {
            self.skip_whitespace_and_comments();
            if self.cursor >= self.data.len() {
                break;
            }

            let b = self.data[self.cursor];

            // 1. Literal string `(...)`
            if b == b'(' {
                let s = self.read_literal_string()?;
                operands.push(PdfObject::String(s));
                continue;
            }

            // 2. Hex string `<...>` or Dict Start `<<`
            if b == b'<' {
                if self.cursor + 1 < self.data.len() && self.data[self.cursor + 1] == b'<' {
                    self.cursor += 2;
                    let dict = self.read_inline_dict(0)?;
                    operands.push(PdfObject::Dictionary(dict));
                    continue;
                }
                let s = self.read_hex_string()?;
                operands.push(PdfObject::String(s));
                continue;
            }

            // 3. Array `[...]`
            if b == b'[' {
                self.cursor += 1;
                let arr = self.read_inline_array(0)?;
                operands.push(PdfObject::Array(arr));
                continue;
            }

            // 4. Name `/Name`
            if b == b'/' {
                let name = self.read_name()?;
                operands.push(PdfObject::Name(name));
                continue;
            }

            // 5. Regular token: Number or Operator
            let tok = self.read_word();
            if tok.is_empty() {
                self.cursor += 1;
                continue;
            }

            // Check if number
            if let Ok(int_val) = std::str::from_utf8(tok).unwrap_or("").parse::<i64>() {
                operands.push(PdfObject::Integer(int_val));
                continue;
            }
            if let Ok(real_val) = std::str::from_utf8(tok).unwrap_or("").parse::<f64>() {
                operands.push(PdfObject::Real(real_val));
                continue;
            }

            // Check boolean
            if tok == b"true" {
                operands.push(PdfObject::Boolean(true));
                continue;
            }
            if tok == b"false" {
                operands.push(PdfObject::Boolean(false));
                continue;
            }
            if tok == b"null" {
                operands.push(PdfObject::Null);
                continue;
            }

            // Otherwise, it is an operator!
            let op_name = String::from_utf8_lossy(tok).to_string();
            let current_operands = std::mem::take(&mut operands);
            operations.push(Operation::new(op_name, current_operands));
        }

        Ok(operations)
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            if Lexer::is_whitespace(b) {
                self.cursor += 1;
            } else if b == b'%' {
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

    fn read_word(&mut self) -> &'a [u8] {
        let start = self.cursor;
        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            if Lexer::is_whitespace(b) || Lexer::is_delimiter(b) {
                break;
            }
            self.cursor += 1;
        }
        &self.data[start..self.cursor]
    }

    fn read_name(&mut self) -> PdfResult<PdfName> {
        self.cursor += 1; // skip '/'
        let start = self.cursor;
        while self.cursor < self.data.len() {
            let b = self.data[self.cursor];
            if Lexer::is_whitespace(b) || Lexer::is_delimiter(b) {
                break;
            }
            self.cursor += 1;
        }
        let raw = String::from_utf8_lossy(&self.data[start..self.cursor]).to_string();
        Ok(PdfName::new(raw))
    }

    fn read_literal_string(&mut self) -> PdfResult<PdfString> {
        let mut lex = Lexer::at_offset(self.data, self.cursor);
        match lex.next_token()? {
            Some(Token::LiteralString(s)) => {
                self.cursor = lex.cursor();
                Ok(s)
            }
            _ => Err(PdfError::ContentStreamError("Failed to parse literal string".to_string())),
        }
    }

    fn read_hex_string(&mut self) -> PdfResult<PdfString> {
        let mut lex = Lexer::at_offset(self.data, self.cursor);
        match lex.next_token()? {
            Some(Token::HexString(s)) => {
                self.cursor = lex.cursor();
                Ok(s)
            }
            _ => Err(PdfError::ContentStreamError("Failed to parse hex string".to_string())),
        }
    }

    fn read_inline_array(&mut self, depth: usize) -> PdfResult<Vec<PdfObject>> {
        if depth >= MAX_CONTENT_NESTING {
            return Err(PdfError::ContentStreamError(
                "Content stream nesting is too deep".to_string(),
            ));
        }
        let mut items = Vec::new();
        while self.cursor < self.data.len() {
            self.skip_whitespace_and_comments();
            if self.cursor >= self.data.len() {
                break;
            }
            if self.data[self.cursor] == b']' {
                self.cursor += 1;
                break;
            }

            // A delimiter that is not a value must still move the cursor.
            // `[/` and `[)` used to stay on the same byte and never return.
            let before = self.cursor;
            if let Some(value) = self.read_inline_value(depth)? {
                items.push(value);
            }
            if self.cursor == before {
                self.cursor += 1;
            }
        }
        Ok(items)
    }

    fn read_inline_dict(&mut self, depth: usize) -> PdfResult<crate::cos::object::PdfDictionary> {
        if depth >= MAX_CONTENT_NESTING {
            return Err(PdfError::ContentStreamError(
                "Content stream nesting is too deep".to_string(),
            ));
        }
        let mut dict = crate::cos::object::PdfDictionary::new();
        while self.cursor < self.data.len() {
            self.skip_whitespace_and_comments();
            if self.cursor >= self.data.len() {
                break;
            }
            if self.cursor + 1 < self.data.len() && &self.data[self.cursor..self.cursor + 2] == b">>" {
                self.cursor += 2;
                break;
            }
            if self.data[self.cursor] == b'/' {
                let key = self.read_name()?;
                self.skip_whitespace_and_comments();
                let before = self.cursor;
                if let Some(value) = self.read_inline_value(depth)? {
                    dict.insert(key, value);
                }
                if self.cursor == before {
                    self.cursor += 1;
                }
            } else {
                self.cursor += 1;
            }
        }
        Ok(dict)
    }

    /// Reads one inline value. Returns `None` when the next byte is not a value.
    /// The caller advances if this function leaves the cursor where it was.
    fn read_inline_value(&mut self, depth: usize) -> PdfResult<Option<PdfObject>> {
        if self.cursor >= self.data.len() {
            return Ok(None);
        }
        let b = self.data[self.cursor];
        if b == b'(' {
            return Ok(Some(PdfObject::String(self.read_literal_string()?)));
        }
        if b == b'<' {
            if self.cursor + 1 < self.data.len() && self.data[self.cursor + 1] == b'<' {
                self.cursor += 2;
                return Ok(Some(PdfObject::Dictionary(self.read_inline_dict(depth + 1)?)));
            }
            return Ok(Some(PdfObject::String(self.read_hex_string()?)));
        }
        if b == b'[' {
            self.cursor += 1;
            return Ok(Some(PdfObject::Array(self.read_inline_array(depth + 1)?)));
        }
        if b == b'/' {
            return Ok(Some(PdfObject::Name(self.read_name()?)));
        }
        let word = self.read_word();
        if word.is_empty() {
            return Ok(None);
        }
        let text = std::str::from_utf8(word).unwrap_or("");
        if let Ok(int_val) = text.parse::<i64>() {
            return Ok(Some(PdfObject::Integer(int_val)));
        }
        if let Ok(real_val) = text.parse::<f64>() {
            return Ok(Some(PdfObject::Real(real_val)));
        }
        Ok(None)
    }
}

/// Converts a flat sequence of operations into a structured `ContentAst`.
pub fn build_ast_from_operations(operations: Vec<Operation>) -> ContentAst {
    let mut ast = ContentAst::new();
    let mut i = 0;

    while i < operations.len() {
        let op = &operations[i];

        if op.operator == "BT" {
            // Collect all operations until matching "ET"
            let id = ast.alloc_id();
            let mut text_ops = vec![op.clone()];
            i += 1;

            while i < operations.len() {
                let inner = &operations[i];
                text_ops.push(inner.clone());
                if inner.operator == "ET" {
                    i += 1;
                    break;
                }
                i += 1;
            }

            ast.nodes.push(ContentNode::TextBlock {
                id,
                operations: text_ops,
            });
        } else if op.operator == "q" {
            // Collect graphics state block until matching "Q"
            let id = ast.alloc_id();
            let mut group_ops = Vec::new();
            i += 1;
            let mut depth = 1;

            while i < operations.len() {
                let inner = &operations[i];
                if inner.operator == "q" {
                    depth += 1;
                } else if inner.operator == "Q" {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                group_ops.push(inner.clone());
                i += 1;
            }

            let nested_ast = build_ast_from_operations(group_ops);
            ast.nodes.push(ContentNode::GraphicsGroup {
                id,
                children: nested_ast.nodes,
            });
        } else {
            let id = ast.alloc_id();
            ast.nodes.push(ContentNode::Instruction {
                id,
                operation: op.clone(),
            });
            i += 1;
        }
    }

    ast
}

/// Serializes a `ContentAst` back to raw PDF Content Stream operator bytes.
pub fn serialize_ast(ast: &ContentAst) -> Vec<u8> {
    let mut out = Vec::new();
    serialize_nodes(&ast.nodes, &mut out);
    out
}

fn serialize_nodes(nodes: &[ContentNode], out: &mut Vec<u8>) {
    for node in nodes {
        match node {
            ContentNode::TextBlock { operations, .. } => {
                for op in operations {
                    serialize_operation(op, out);
                }
            }
            ContentNode::GraphicsGroup { children, .. } => {
                out.extend_from_slice(b"q\n");
                serialize_nodes(children, out);
                out.extend_from_slice(b"Q\n");
            }
            ContentNode::Instruction { operation, .. } => {
                serialize_operation(operation, out);
            }
        }
    }
}

fn serialize_operation(op: &Operation, out: &mut Vec<u8>) {
    for operand in &op.operands {
        let mut buf = Vec::new();
        let mut writer = crate::cos::Writer::new(&mut buf);
        let _ = writer.write_object(operand);
        out.extend_from_slice(&buf);
        out.push(b' ');
    }
    out.extend_from_slice(op.operator.as_bytes());
    out.push(b'\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cos::object::PdfObject;

    #[test]
    fn test_content_stream_tokenization_and_ast() {
        let stream = b"q\n1 0 0 1 50 100 cm\nBT\n/F1 12 Tf\n10 20 Td\n(Hello World) Tj\nET\nQ\n";
        let mut tokenizer = ContentStreamTokenizer::new(stream);
        let ops = tokenizer.tokenize_all().unwrap();

        assert_eq!(ops.len(), 8); // q, cm, BT, Tf, Td, Tj, ET, Q
        let ast = build_ast_from_operations(ops);

        assert_eq!(ast.nodes.len(), 1); // Top-level GraphicsGroup
        match &ast.nodes[0] {
            ContentNode::GraphicsGroup { children, .. } => {
                assert_eq!(children.len(), 2); // cm instruction + TextBlock
                match &children[1] {
                    ContentNode::TextBlock { operations, .. } => {
                        assert_eq!(operations.len(), 5); // BT, Tf, Td, Tj, ET
                    }
                    _ => panic!("Expected TextBlock inside GraphicsGroup"),
                }
            }
            _ => panic!("Expected GraphicsGroup"),
        }

        // Test roundtrip serialization
        let reserialized = serialize_ast(&ast);
        let text = String::from_utf8_lossy(&reserialized);
        assert!(text.contains("(Hello World) Tj"));
        assert!(text.contains("/F1 12 Tf"));
    }

    #[test]
    fn inline_array_reads_names_strings_and_numbers() {
        let stream = b"[/DeviceRGB] CS\n[(Hello) -10 2.5] TJ\n";
        let mut tokenizer = ContentStreamTokenizer::new(stream);
        let ops = tokenizer.tokenize_all().expect("inline array");

        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].operator, "CS");
        match &ops[0].operands[0] {
            PdfObject::Array(items) => {
                assert_eq!(items[0].as_name(), Some("DeviceRGB"));
            }
            other => panic!("expected array, found {other:?}"),
        }
        assert_eq!(ops[1].operator, "TJ");
        match &ops[1].operands[0] {
            PdfObject::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0].as_string_bytes(), Some(b"Hello".as_slice()));
                assert_eq!(items[1].as_i64(), Some(-10));
                assert_eq!(items[2].as_f64(), Some(2.5));
            }
            other => panic!("expected array, found {other:?}"),
        }
    }

    #[test]
    fn delimiter_inside_an_array_does_not_stick() {
        let stream = b"[) > { }] TJ\n[/Name] scn\n";
        let mut tokenizer = ContentStreamTokenizer::new(stream);
        let ops = tokenizer.tokenize_all().expect("delimiters advance");
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].operator, "TJ");
        assert_eq!(ops[1].operator, "scn");
    }

    #[test]
    fn deeply_nested_arrays_are_rejected() {
        let stream = vec![b'['; MAX_CONTENT_NESTING + 2];
        let mut tokenizer = ContentStreamTokenizer::new(&stream);
        let error = tokenizer.tokenize_all().expect_err("depth cap");
        assert!(error.to_string().contains("nesting is too deep"));
    }
}
