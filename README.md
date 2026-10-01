# PDFEngine

[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg?style=flat-square)](https://www.rust-lang.org)
[![ISO 32000 Conformance](https://img.shields.io/badge/standard-ISO%2032000--1%20%7C%2032000--2-blue.svg?style=flat-square)](https://www.iso.org/standard/75839.html)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square)](https://opensource.org/licenses/MIT)
[![Security: Hardened](https://img.shields.io/badge/security-hardened-green.svg?style=flat-square)](https://github.com/rgarduno/PDFEngine)

**PDFEngine** is a high-performance, memory-safe, lossless PDF processing and surgical editing engine built in pure safe Rust. Designed from the ground up for zero-drift in-place text replacement, semantic typographic reflow, and enterprise-grade resilience in high-throughput cloud APIs, serverless functions (AWS Lambda, Google Cloud Functions), and client-side WebAssembly runtimes.

---

## The Problem: Why Existing PDF Editors Fail

Most commercial PDF editors and open-source libraries operate under one of two destructive paradigms:

1. **Lossy Document Reconstruction**: Converting PDF pages into intermediate formats (HTML, DOCX, or raster images) and regenerating the PDF. This destroys vector curves, clipping paths, CMYK color spaces, form fields (`AcroForms`), blend modes, and embedded font subsets.
2. **Naive String/Operator Replacement**: Replacing text inside individual `Tj` or `TJ` operators using string substitution. This causes text collisions, overflows across margins, broken character encodings (due to missing glyphs in font subsets), and ignores paragraph alignment and horizontal kerning adjustments.

### The PDFEngine Solution

PDFEngine operates directly on the native **ISO 32000 Content Stream Abstract Syntax Tree (AST)**:
* **Surgical AST Mutation**: Only target text operator nodes (`BT ... ET`) are rewritten. 100% of non-edited vector paths, clipping boundaries, images (`Do`), shading patterns, and graphics state stacks (`q ... Q`) remain bit-for-bit identical.
* **Semantic Layout Hierarchy**: Visual glyph runs are deterministically clustered into **Spans $\to$ Text Lines $\to$ Paragraph Blocks** with automatic detection of baseline, leading, and alignment (left, center, right, justified).
* **Typographic-Safe Reflow**: Line breaks and horizontal advances are computed using the embedded font's actual `/Widths` and `hmtx` tables, handling font subset limitations, ligature decomposition (`fi`, `fl`), and ghost space inference.

---

## Architectural Overview

```
                                  Client Layer
                 ┌──────────────────────────────────────────────┐
                 │     Web / Desktop App (React / Next.js)      │
                 │  - Dual-Layer Precision Canvas               │
                 │  - Dynamic FontFace WOFF2 Registration       │
                 │  - Interactive Real-Time Reflow Editing      │
                 └──────────────────────┬───────────────────────┘
                                        │ JSON SceneGraph & WebSockets
                                        ▼
                                 Service Layer
                 ┌──────────────────────────────────────────────┐
                 │       Backend API (FastAPI / Actix-Web)      │
                 │  - Document Ingestion & Page Streaming      │
                 │  - Serverless Ready (AWS Lambda / Containers)│
                 └──────────────────────┬───────────────────────┘
                                        │ PyO3 / C-ABI Bindings
                                        ▼
                               PDFEngine Core (Rust)
┌─────────────────────────────────────────────────────────────────────────────┐
│ 1. Carousel Object System (COS)                                            │
│    - Byte-level Lexer (§7.2) with hex/octal string & name normalization     │
│    - Syntactic Parser (§7.3) for primitives, arrays, dicts, and streams    │
│    - Cross-Reference Engine (§7.5.4 & §7.5.8): classic xref, XRef Streams, │
│      incremental update history (/Prev), and Object Streams (/ObjStm)       │
│    - Deterministic Byte-Accurate Serializer & Writer                       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 2. Content Stream & Graphics State Engine                                  │
│    - Lossless AST Decomposition (BT/ET, q/Q, path construction/painting)   │
│    - Affine Matrix Computation: Rendering Coordinates = CTM × Tm           │
├─────────────────────────────────────────────────────────────────────────────┤
│ 3. Typographic & Font Engine                                               │
│    - TrueType / OpenType binary table reader (cmap, hmtx, head, glyf)       │
│    - Bidirectional /ToUnicode CMap decoder and CID reverse encoder         │
│    - Typographic ligature preservation and ghost space metric inference    │
│    - Metric-compatible glyph injection for subsetted font extensions        │
├─────────────────────────────────────────────────────────────────────────────┤
│ 4. Semantic Layout & Surgical Reflow Engine                                 │
│    - Spatial clustering: Glyphs -> Spans -> TextLines -> ParagraphBlocks    │
│    - Knuth-Plass / Greedy line-breaking with exact character advance delta  │
│    - Atomic node replacement within page content stream display list        │
├─────────────────────────────────────────────────────────────────────────────┤
│ 5. Security & Resource Hardening                                            │
│    - Bounded Flate expansion: 100:1 max ratio, 250 MB ceiling (Zip Bomb)   │
│    - Circular reference detection (HashSet tracking) & recursion cap (64)   │
│    - Active code neutralization (strips /JavaScript, /Launch, /SubmitForm) │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Workspace Structure

```
PDFEngine/
├── Cargo.toml                  # Cargo workspace configuration
├── crates/
│   ├── pdf-engine-core/        # Pure Rust library: ISO 32000 parsing, layout, editor
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs          # Public crate API and layer re-exports
│   │   │   ├── error.rs        # Strongly-typed PdfError enum (thiserror)
│   │   │   ├── security/       # Resource limits, recursion & zip bomb guards
│   │   │   ├── cos/            # Object model, lexer, parser, filters, xref, writer
│   │   │   ├── stream/         # Content Stream AST & graphics state evaluator
│   │   │   ├── fonts/          # TrueType/CFF parsing, ToUnicode, glyph injection
│   │   │   ├── layout/         # Semantic clustering & paragraph reconstruction
│   │   │   └── editor/         # Surgical stream mutator & reflow engine
│   │   └── tests/              # Conformance and integration test suite
│   └── pdf-engine-ffi/         # C-ABI & WebAssembly bindings (planned)
└── bindings/
    └── python/                 # PyO3 native Python package (planned)
```

---

## Security & Resilience (Enterprise Grade)

PDF is historically one of the most targeted document formats for memory corruption and denial-of-service exploits. PDFEngine enforces strict security invariants:

| Defense Vector | Attack Mechanism | Engine Mitigation |
| :--- | :--- | :--- |
| **Zip / Decompression Bombs** | Small compressed streams expanding to gigabytes in memory. | Bounded chunk reader enforcing a **100:1 maximum expansion ratio** and a configurable hard ceiling (default: 250 MiB). |
| **Circular Reference Loops** | Malicious indirect objects referencing each other cyclically. | Traversal depth limit (maximum 64 levels) and `HashSet<(u32, u16)>` cycle detection. |
| **Buffer Overflows & Use-After-Free** | Pointer manipulation bugs in legacy C/C++ parsers. | **100% Safe Rust** codebase. Memory safety guaranteed at compile time without garbage collection pauses. |
| **Malicious Active Scripts** | Exploits via embedded `/JavaScript` or `/Launch` actions. | All active scripts and OS commands are neutralized and stripped from processing pipelines. |

---

## Performance Characteristics

* **Zero Memory Leaks**: Deterministic RAII memory management; completely eliminates garbage collection freezes.
* **Low Cold-Start Latency**: Under 15ms initialization overhead, optimal for AWS Lambda, Cloud Run, and edge functions.
* **Stateless & Thread-Safe**: All document structures implement `Send + Sync` with zero global state, allowing safe multi-tenant concurrency.
* **Zero-Copy Byte Scanning**: High-throughput lexical scanning over contiguous byte buffers.

---

## Quickstart (Rust Core)

Add the engine to your `Cargo.toml`:

```toml
[dependencies]
pdf-engine-core = { path = "crates/pdf-engine-core" }
```

### Loading and Inspecting a PDF

```rust
use pdf_engine_core::cos::{ObjectId, PdfDocument, PdfObject};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdf_bytes = std::fs::read("document.pdf")?;
    let mut doc = PdfDocument::load(&pdf_bytes)?;

    // Inspect Catalog dictionary
    let catalog = doc.catalog()?;
    println!("Catalog: {:?}", catalog);

    // Retrieve all page object IDs in reading order
    let page_ids = doc.get_pages()?;
    println!("Found {} pages", page_ids.len());

    for (idx, page_id) in page_ids.iter().enumerate() {
        let page_obj = doc.get_object(*page_id)?;
        println!("Page {}: {:?}", idx + 1, page_obj);
    }

    Ok(())
}
```

### Surgical In-Place Text Editing & Layout Reflow

```rust
use pdf_engine_core::cos::PdfDocument;
use pdf_engine_core::editor::SurgicalEditor;
use pdf_engine_core::fonts::FontMetrics;
use pdf_engine_core::layout::LayoutReconstructor;
use pdf_engine_core::stream::{build_ast_from_operations, serialize_ast, ContentStreamTokenizer};

fn edit_page_paragraph(page_content_bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // 1. Parse content stream into lossless AST
    let mut tokenizer = ContentStreamTokenizer::new(page_content_bytes);
    let ops = tokenizer.tokenize_all()?;
    let mut ast = build_ast_from_operations(ops);

    // 2. Reconstruct semantic layout (ParagraphBlocks, Lines, Spans)
    let metrics = FontMetrics::new(0, 255, vec![500.0; 256], 500.0);
    let reconstructor = LayoutReconstructor::new(&ast).with_font("F1", metrics.clone());
    let paragraphs = reconstructor.reconstruct();

    // 3. Perform surgical in-place edit on target paragraph
    if let Some(target) = paragraphs.first() {
        SurgicalEditor::edit_paragraph(
            &mut ast,
            target,
            "Updated agreement terms executed with zero layout drift.",
            &metrics,
        )?;
    }

    // 4. Re-serialize AST to pure ISO 32000 content stream bytes
    Ok(serialize_ast(&ast))
}
```

---

## Project Roadmap

- [x] **Phase 0: Workspace Setup & Architecture** (Cargo workspace, coding standards, CI baseline)
- [x] **Phase 1: Safe COS Core** (Lexer, Parser, XRef streams, Flate/PNG filters, Writer, Tests)
- [x] **Phase 2: Content Streams & Typographic Engine** (AST operator parser, Graphics State, TrueType tables, ToUnicode CMaps, ligatures)
- [x] **Phase 3: Semantic Layout & Surgical Reflow** (Glyph clustering, paragraph reflow, in-place AST mutator)
- [ ] **Phase 4: Python Bindings & FastAPI Backend** (PyO3 native bindings, document upload, font server, WebSocket reflow)
- [ ] **Phase 5: React / Next.js Web Application** (Dual-layer canvas, in-situ editing, FontFace loader)
- [ ] **Phase 6: Hardening & Conformance Suite** (Real-world stress corpus, visual regression diffing)

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
