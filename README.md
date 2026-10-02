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
│ 5. XObject Image & Graphics Engine                                          │
│    - Safe ISO 32000 XObject /Image extraction & CTM transformation parsing  │
│    - Pure W3C PNG & JPEG header parsers with /SMask soft mask extraction    │
│    - Surgical in-place image swapping preserving page geometry & vectors    │
├─────────────────────────────────────────────────────────────────────────────┤
│ 6. Interactive AcroForms & Form Flattening Engine                           │
│    - ISO 32000-1 §12.7 AcroForms reader & field hierarchy traversal         │
│    - Text, Checkbox, Radio, and Choice field filling with appearance (/AP)  │
│    - Surgical form flattening burning values into page vector streams       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 7. Security & Resource Hardening                                            │
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
│   │   │   ├── images/         # XObject Image extraction, JPEG/PNG codecs & surgical replacement
│   │   │   ├── forms/          # AcroForms reader, field filler & surgical flattening
│   │   │   └── editor/         # Surgical stream mutator & reflow engine
│   │   └── tests/              # Conformance and integration test suite
│   └── pdf-engine-python/      # High-performance PyO3 native Python extension
│       ├── Cargo.toml
│       └── src/lib.rs          # PyPdfDocument, PyPage, PyParagraph, PyFormField exports
├── backend/                    # Commercial FastAPI REST & WebSocket service
│   ├── app/
│   │   ├── main.py             # REST endpoints & real-time WebSocket reflow channel
│   │   └── models.py           # Pydantic v2 schemas (SceneGraph, BoundingBox, Edits)
│   ├── requirements.txt
│   └── tests/
│       └── test_api.py         # Full HTTP & WebSocket integration test suite
└── web/                        # Next.js 16 + React 19 Interactive Studio
    ├── src/
    │   ├── app/                # App router studio entrypoint
    │   ├── components/         # DualCanvasViewer, Toolbar, Sidebar
    │   └── lib/                # API client, WebSocket stream & TypeScript types
    └── package.json
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

## Commercial Python & FastAPI Service

PDFEngine compiles to a native Python C-extension via PyO3, with a production-grade FastAPI microservice ready for containerized or serverless deployment.

### 1. Direct Python API Usage

```python
import pdf_engine

# Load document from file or bytes
doc = pdf_engine.Document.load("contract.pdf")
print(f"Total pages: {doc.page_count()}")

# Inspect page layout scenegraph
page = doc.get_page(1)
paragraphs = page.get_paragraphs()

for p in paragraphs:
    print(f"Paragraph {p.id}: [{p.alignment}] {p.text[:40]}... (bbox: {p.bbox()})")

# Surgically edit target paragraph
page.edit_paragraph(0, "Amended Terms Approved with zero layout drift.")
doc.update_page(page)

# Export modified PDF
doc.save("contract_edited.pdf")
```

### 2. Running the FastAPI Service

```bash
# Setup environment & install dependencies
python3 -m venv backend/.venv
backend/.venv/bin/pip install -r backend/requirements.txt

# Start production server
PYTHONPATH=backend backend/.venv/bin/uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
```

### 3. API Endpoints

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/api/health` | Health check and native engine availability. |
| `POST` | `/api/documents/upload` | Ingest PDF, validate ISO structure, and return session token. |
| `GET` | `/api/documents/{id}/pages/{p}/scenegraph` | Retrieve semantic layout (paragraphs, bounding boxes, alignments). |
| `GET` | `/api/documents/{id}/pages/{p}/fonts` | List embedded font resources declared on a specific page. |
| `GET` | `/api/documents/{id}/pages/{p}/fonts/{name}` | Stream raw embedded TrueType/OpenType font binary for browser `@font-face` registration. |
| `GET` | `/api/documents/{id}/pages/{p}/images` | List XObject images on a specific page with CTM bounding boxes and metadata. |
| `GET` | `/api/documents/{id}/images/{img_id}` | Stream synthesized PNG or native JPEG binary for inspection/preview. |
| `POST` | `/api/documents/{id}/images/{img_id}/replace` | Surgical in-place image replacement (JPEG/PNG with `/SMask` transparency). |
| `POST` | `/api/documents/{id}/pages/{p}/edit/{para_id}` | Surgical in-place paragraph text replacement with auto-reflow. |
| `GET` | `/api/documents/{id}/forms` | List all interactive AcroForm fields, types, options, and current values. |
| `POST` | `/api/documents/{id}/forms/fill` | Fill field value (text, checkbox, choice) with auto-synthesized `/AP /N` appearances. |
| `POST` | `/api/documents/{id}/forms/flatten` | Surgically burn all form field values into page `/Contents` and purge `/AcroForm`. |
| `GET` | `/api/documents/{id}/export` | Download finalized modified PDF with bit-for-bit preserved vector graphics. |
| `WS` | `/ws/documents/{id}/pages/{p}/reflow` | Real-time WebSocket channel streaming live layout reflow as user types. |

---

## Interactive Web Studio (Next.js 16 + React 19)

PDFEngine includes a modern, high-precision web studio inside `web/` with a dual-layer canvas architecture:

* **Dual-Layer Canvas Viewport**: Renders the document canvas with accurate page points and overlays interactive paragraph bounding boxes.
* **Layer 1.5 Image Overlays & Replacement**: Visual inspection of XObject images with floating action buttons for instant in-place PNG/JPEG swapping.
* **Layer 1.8 Interactive AcroForms & Flattening**: In-situ filling for text inputs, checkboxes, and select dropdowns, coupled with single-click surgical document flattening.
* **Dynamic @font-face Registration**: Fetches embedded TrueType font binaries directly from the PDF via the engine and registers them in the browser runtime for pixel-identical typography.
* **In-Situ Typographic Editor**: Double-click any paragraph to edit directly in place with true-to-life baseline alignment and leading.
* **Live WebSocket Reflow**: Bidirectional communication with the Rust engine recalculates line wraps and bounding box expansions with zero visual lag.
* **Non-Destructive History**: Full undo/redo stack (`Cmd+Z` / `Cmd+Shift+Z`) and instant lossless PDF download.

### Running the Web Studio

```bash
cd web
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) to start editing.

---

## Project Roadmap

- [x] **Phase 0: Workspace Setup & Architecture** (Cargo workspace, coding standards, CI baseline)
- [x] **Phase 1: Safe COS Core** (Lexer, Parser, XRef streams, Flate/PNG filters, Writer, Tests)
- [x] **Phase 2: Content Streams & Typographic Engine** (AST operator parser, Graphics State, TrueType tables, ToUnicode CMaps, ligatures)
- [x] **Phase 3: Semantic Layout & Surgical Reflow** (Glyph clustering, paragraph reflow, in-place AST mutator)
- [x] **Phase 4: Python Bindings & FastAPI Backend** (PyO3 native bindings, document upload, scene graph inspection, surgical edit endpoints, WebSocket reflow)
- [x] **Phase 5: React / Next.js Web Application** (Dual-layer canvas, in-situ editing, live WebSocket reflow)
- [x] **Phase 6: Hardening & Conformance Suite** (Real-world stress corpus, visual regression diffing, zip bomb mitigation, circular reference loop prevention)
- [x] **Phase 7: Embedded Font Extraction & Dynamic Glyph Fallback** (WinAnsi encoding, Spanish/Latin-1 accents, metric transliteration fallback, TrueType font streaming)
- [x] **Phase 8: Graphics & XObject Image Management** (CTM spatial projection, pure JPEG/PNG codecs, in-place surgical replacement, web studio inspection & replacement)
- [x] **Phase 9: Interactive AcroForms & Surgical Form Flattening** (AcroForm hierarchy reader, appearance synthesis, surgical flattening, PyO3 bindings, FastAPI endpoints & Web Studio)

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
