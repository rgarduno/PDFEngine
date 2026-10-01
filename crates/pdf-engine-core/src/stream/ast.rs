//! Content Stream Abstract Syntax Tree (AST) according to ISO 32000-1 §7.8.
//!
//! Provides a lossless, non-destructive tree representation of page content operations.
//! Preserves operator order and allows surgical in-place modification of text blocks
//! without corrupting vector paths or graphics states.

use crate::cos::object::PdfObject;

/// Unique identifier for an AST operation node within a page content stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub usize);

/// An individual operator instruction within a PDF content stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// Postfix operator identifier (e.g. "BT", "ET", "Tf", "TJ", "cm", "q", "Q").
    pub operator: String,
    /// Operands supplied to this operator.
    pub operands: Vec<PdfObject>,
}

impl Operation {
    /// Creates a new operation with an operator name and operands.
    pub fn new(operator: impl Into<String>, operands: Vec<PdfObject>) -> Self {
        Self {
            operator: operator.into(),
            operands,
        }
    }
}

/// Abstract Syntax Tree node representing a hierarchical sequence of page operations.
#[derive(Debug, Clone, PartialEq)]
pub enum ContentNode {
    /// Text object block delimited by `BT` and `ET` operators.
    TextBlock {
        id: NodeId,
        operations: Vec<Operation>,
    },
    /// Graphics state block delimited by `q` and `Q` operators.
    GraphicsGroup {
        id: NodeId,
        children: Vec<ContentNode>,
    },
    /// Standalone path, color, or XObject operator outside text blocks.
    Instruction {
        id: NodeId,
        operation: Operation,
    },
}

/// A parsed content stream tree representing the entire visual display list of a page.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContentAst {
    /// Ordered sequence of top-level AST nodes.
    pub nodes: Vec<ContentNode>,
    /// Counter for generating unique node identifiers.
    next_node_id: usize,
}

impl ContentAst {
    /// Creates an empty content AST.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates a new unique `NodeId`.
    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        id
    }
}
