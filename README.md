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
│ 6. Annotations, Interactive Links & Rubber Stamps                           │
│    - ISO 32000-1 §12.5 Annotation reader & QuadPoints geometry evaluation   │
│    - Text markups (/Highlight, /Underline, /StrikeOut) with synthetic /AP   │
│    - Interactive external Web URIs and internal /GoTo page destinations     │
│    - Vector rubber stamps with dual-border styling & customizable rubrics   │
│    - Surgical annotation flattening into permanent page content streams     │
├─────────────────────────────────────────────────────────────────────────────┤
│ 7. Interactive AcroForms & Form Flattening Engine                           │
│    - ISO 32000-1 §12.7 AcroForms reader & field hierarchy traversal         │
│    - Text, Checkbox, Radio, and Choice field filling with appearance (/AP)  │
│    - Surgical form flattening burning values into page vector streams       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 8. Document Assembly & Structural Operations                                │
│    - Transitive object graph cloning with cycle prevention                  │
│    - Page rotation (0°, 90°, 180°, 270°) with canonical normalization      │
│    - Document splitting by page ranges and fixed-size chunking              │
│    - Multi-document concatenation and merging preserving resources & fonts  │
│    - In-place page reordering and deletion with single-page safety guards   │
├─────────────────────────────────────────────────────────────────────────────┤
│ 9. Dynamic Pagination & Semitransparent Watermarks                          │
│    - Bates numbering & headers/footers with {page} and {total} templating   │
│    - Semitransparent text watermarks with matrix rotation and /ExtGState /ca│
│    - Embedded image watermarks (PNG/JPEG) with background/foreground depth  │
├─────────────────────────────────────────────────────────────────────────────┤
│ 10. Glyph excision and PII scan (not a legal redaction)                     │
│    - Physical glyph & stream excision from AST (no visual-only hiding)      │
│    - Zero layout shift: coordinates of non-redacted text preserved via Tm   │
│    - Automated PII scanning: Email, Phone, RFC, CURP, Credit Card (Luhn)    │
│    - Opaque blackout vector patches (`re f`) with centered overlay labels   │
│    - Interactive annotation pruning (/Link, /Highlight leaks prevented)    │
│    - Metadata /Info and XMP are removed only when scrub_metadata is set     │
├─────────────────────────────────────────────────────────────────────────────┤
│ 11. PDF Security, Permissions & SHA-256 Attestation (ISO 32000 §7.6 & §12.8)│
│    - Rev 4 security handler: AES-128 CBC, SHA-256, and MD5                  │
│    - Standard Security Handler Rev 4 (AES-128) with /O, /U and /Perms       │
│    - /P permission bits are stored and are not enforced by this process     │
│    - Digital signatures with AcroForm /Sig fields and /ByteRange validation │
├─────────────────────────────────────────────────────────────────────────────┤
│ 12. Structured Table Reconstruction & Semantic Extraction (ISO 32000 §14.8.4│
│    - Vector lattice grid solver with path projection and collinear merging  │
│    - Stream/borderless fallback via whitespace clustering & margin alignment│
│    - Precision paragraph & span association with cell containment threshold │
│    - Multi-format exporters: RFC 4180 CSV, JSON, Markdown, and semantic HTML│
├─────────────────────────────────────────────────────────────────────────────┤
│ 13. Security & Resource Hardening                                           │
│    - Bounded Flate expansion: 100:1 max ratio, 256 MiB ceiling (Zip Bomb)   │
│    - Circular reference detection (HashSet tracking) & recursion cap (64)   │
│    - Strips /JavaScript, /JS, /Launch, /SubmitForm, /OpenAction, and /AA    │
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
│   │   │   ├── crypto/         # AES-128 CBC, SHA-256, and MD5 primitives
│   │   │   ├── security/       # Standard Security Handler Rev 4, permissions & signatures
│   │   │   ├── cos/            # Object model, lexer, parser, filters, xref, writer
│   │   │   ├── stream/         # Content Stream AST & graphics state evaluator
│   │   │   ├── fonts/          # TrueType/CFF parsing, ToUnicode, glyph injection
│   │   │   ├── layout/         # Semantic clustering & paragraph reconstruction
│   │   │   ├── images/         # XObject Image extraction, JPEG/PNG codecs & surgical replacement
│   │   │   ├── forms/          # AcroForms reader, field filler & surgical flattening
│   │   │   ├── annots/         # ISO 32000-1 annotations: markups, links, stamps & flattening
│   │   │   ├── ops/            # Document operations: cloner, rotation, split, merge, reorder, delete
│   │   │   ├── watermark/      # Dynamic Bates pagination, headers/footers & semitransparent watermarks
│   │   │   ├── redact/         # Glyph excision and PII scan; metadata scrub is optional
│   │   │   ├── tables/         # ISO 32000-1 §14.8.4 table detection & multi-format export
│   │   │   └── editor/         # Surgical stream mutator & reflow engine
│   │   └── tests/              # Conformance and integration test suite
│   └── pdf-engine-python/      # High-performance PyO3 native Python extension
│       ├── Cargo.toml
│       └── src/lib.rs          # PyPdfDocument, PyPage, PyParagraph, PyFormField, PyAnnotation, PyRedaction, PyTable exports
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
| **Zip / Decompression Bombs** | Small compressed streams expanding to gigabytes in memory. | Bounded chunk reader enforcing a **100:1 maximum expansion ratio** and a hard ceiling of 256 MiB. A compressed input of 1 KiB or less may exceed that ratio until the output passes 1 MiB. An empty compressed stream that yields output is rejected. PNG image data uses the same ceiling, and a predictor `Columns` of 0 is rejected. |
| **Circular Reference Loops** | Malicious indirect objects referencing each other cyclically. | Traversal depth limit (maximum 64 levels) and `HashSet<(u32, u16)>` cycle detection. |
| **Buffer Overflows & Use-After-Free** | Pointer manipulation bugs in legacy C/C++ parsers. | **100% Safe Rust** codebase. Memory safety guaranteed at compile time without garbage collection pauses. |
| **Malicious Active Scripts** | Exploits via embedded `/JavaScript` or `/Launch` actions. | Save removes `/JavaScript`, `/JS`, `/Launch`, `/SubmitForm`, `/OpenAction`, and `/AA`. `http` and `https` `/URI` links stay; `javascript`, `vbscript`, `file`, and `data` schemes are removed. |
| **Unbounded uploads and sessions** | A client pins process memory by uploading without a limit or by leaving documents open. | Uploads above 32 MiB are rejected (413). The process keeps at most 32 sessions and 256 MiB of accounted file bytes, and each session expires 30 minutes after its last successful load (429). |
| **Object count and optimizer work** | A cross-reference reserves a slot per object number, an object stream declares a huge `/N`, or compressed objects point at each other. | At most 500,000 objects. Cross-reference streams list only occupied numbers. Object-stream `/N` and `/First` outside the decoded stream are rejected. A compressed-object cycle fails closed. Each object stream holds at most 100 objects, and zlib-best recompression skips streams larger than 1 MiB. |
| **Optimizer on protected files** | A size rewrite moves every byte offset. A byte-range signature would no longer match, and encryption is not re-applied. | Optimization is refused (409) when the trailer has `/Encrypt`, or when an object is `/Type /Sig` or `/SubFilter /PDFEngine.sha256`. The file is left unchanged. Words drawn in a content stream are not treated as a signature. |
| **Browser origin and error text** | Any site can call the API with credentials, and a handler returns the engine's internal error text. | `PDFENGINE_CORS_ORIGINS` lists the exact origins that may call the API. Credentials are attached only for an origin on that list. A wildcard is ignored. Clients receive `The request could not be completed.` and the cause stays in the server log. |
| **Downloads, links, permissions, and redaction** | A download name is taken from the upload, a link can use any scheme, `/P` is described as access control, and redaction is described as ISO legal redaction. | Export names are `document_edited.pdf` and `document_optimized.pdf`. New links accept only `http` and `https`. `/P` is stored and not enforced. Redaction removes intersecting glyphs, page `/Metadata`, and marked-content `/ActualText`, `/Alt`, and `/E` on the rewritten page. Attachments, the structure tree, and form appearances stay. Document `/Info` and catalog XMP are removed only when metadata scrubbing is requested. |

---

## Performance Characteristics

* **Zero Memory Leaks**: Deterministic RAII memory management; completely eliminates garbage collection freezes.
* **Low Cold-Start Latency**: Under 15ms initialization overhead, optimal for AWS Lambda, Cloud Run, and edge functions.
* **Bounded sessions**: The Rust document type keeps no process-global state and is `Send`. The API keeps parsed documents in a process-local table capped by upload size (32 MiB), session count (32), accounted bytes (256 MiB), and a sliding 30-minute lifetime.
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
    let paragraphs = reconstructor.reconstruct()?;

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
export PDFENGINE_API_KEYS="replace-with-a-long-random-token"
export PDFENGINE_CORS_ORIGINS="http://localhost:3000"
PYTHONPATH=backend backend/.venv/bin/uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
```

### Authentication

Every route except `GET /api/health` requires `Authorization: Bearer <token>`.
Set `PDFENGINE_API_KEYS` to a comma-separated list of tokens. Each token must
be at least 16 characters. The process stores a SHA-256 subject id with the
document, and a document id can be read only by the subject that created it.
A missing document and a document owned by another subject both answer 404.

The local studio sends `NEXT_PUBLIC_PDFENGINE_API_KEY` on each request.
Next.js inlines that value into the browser bundle, so it is visible to anyone
who can load the studio page. Use it for a single-user studio. A shared
deployment should terminate at a proxy that injects the bearer header and
should not publish a tenant key to the browser.

The studio page at `http://localhost:3000` calls the API on another origin.
`PDFENGINE_CORS_ORIGINS` is a comma-separated list of exact origins, such as
`http://localhost:3000`. A wildcard is ignored. The API attaches credentials
only when the request `Origin` is on that list. An empty list allows no
browser origin.

Browser WebSockets cannot set `Authorization`. `POST /api/auth/ws-ticket`
returns a single-use ticket, valid for 60 seconds, passed as `?ticket=`.

### 3. API Endpoints

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/api/health` | Health check and native engine availability. Public. |
| `POST` | `/api/auth/ws-ticket` | Exchange the bearer token for a single-use WebSocket ticket. |
| `POST` | `/api/documents/upload` | Ingest PDF, validate ISO structure, and return session token. |
| `GET` | `/api/documents/{id}/pages/{p}/scenegraph` | Retrieve semantic layout (paragraphs, bounding boxes, alignments). |
| `GET` | `/api/documents/{id}/pages/{p}/fonts` | List embedded font resources declared on a specific page. |
| `GET` | `/api/documents/{id}/pages/{p}/fonts/{name}` | Stream raw embedded TrueType/OpenType font binary for browser `@font-face` registration. |
| `GET` | `/api/documents/{id}/pages/{p}/images` | List XObject images on a specific page with CTM bounding boxes and metadata. |
| `GET` | `/api/documents/{id}/images/{img_id}` | Stream synthesized PNG or native JPEG binary for inspection/preview. |
| `POST` | `/api/documents/{id}/images/{img_id}/replace` | Surgical in-place image replacement (JPEG/PNG with `/SMask` transparency). |
| `POST` | `/api/documents/{id}/pages/{p}/edit/{para_id}` | Surgical in-place paragraph text replacement with auto-reflow. |
| `GET` | `/api/documents/{id}/forms` | List all interactive AcroForm fields, types, options, and current values. |
| `POST` | `/api/documents/{id}/pages/{p}/forms` | Create and position a new interactive form field (Text, Checkbox, Choice, Signature) on page. |
| `DELETE` | `/api/documents/{id}/forms/{name}` | Delete an interactive form field and its associated widget annotations. |
| `PUT` | `/api/documents/{id}/forms/{name}` | Update form field geometry (bounding box) or flags (read-only, required, multiline). |
| `POST` | `/api/documents/{id}/forms/fill` | Fill field value (text, checkbox, choice) with auto-synthesized `/AP /N` appearances. |
| `POST` | `/api/documents/{id}/forms/flatten` | Surgically burn all form field values into page `/Contents` and purge `/AcroForm`. |
| `POST` | `/api/documents/{id}/pages/{p}/rotate` | Rotate individual page by 90°, 180°, or 270° with ISO `/Rotate` attribute. |
| `POST` | `/api/documents/{id}/split` | Split document into single-page extracts or multi-page chunks. |
| `POST` | `/api/documents/merge` | Concatenate and merge multiple documents into a single unified PDF. |
| `POST` | `/api/documents/{id}/pages/reorder` | Permute page ordering with cycle-safe page tree restructuring. |
| `POST` | `/api/documents/{id}/pages/delete` | Purge pages while enforcing single-page survival safety guards. |
| `GET` | `/api/documents/{id}/pages/{p}/annotations` | List text markups, clickable links, and rubber stamps on a page. |
| `POST` | `/api/documents/{id}/pages/{p}/annotations/markup` | Add Highlight, Underline, or StrikeOut annotation with custom color & opacity. |
| `POST` | `/api/documents/{id}/pages/{p}/annotations/link` | Add interactive clickable Web URI or GoTo page destination. |
| `POST` | `/api/documents/{id}/pages/{p}/annotations/stamp` | Add vector rubber stamp with dual borders and custom rubrics. |
| `DELETE` | `/api/documents/{id}/pages/{p}/annotations/{aid}` | Remove annotation from document tree. |
| `POST` | `/api/documents/{id}/annotations/flatten` | Burn visual annotations into permanent page vector graphics. |
| `POST` | `/api/documents/{id}/pagination` | Apply dynamic Bates numbering & headers/footers with `{page}` and `{total}`. |
| `POST` | `/api/documents/{id}/watermark/text` | Apply semi-transparent rotated text watermark (`/ExtGState /ca`). |
| `POST` | `/api/documents/{id}/watermark/image` | Embed semi-transparent image watermark (PNG/JPEG) with depth placement. |
| `POST` | `/api/documents/{id}/redact/regions` | Surgically excise glyphs in coordinate bounding boxes with blackout patches and overlay label. |
| `POST` | `/api/documents/{id}/redact/pattern` | Scan & permanently excise PII patterns (Email, Phone, RFC, CURP, Credit Card Luhn, SSN). |
| `POST` | `/api/documents/{id}/redact/text` | Search and permanently excise text runs with zero layout shift on non-redacted text. |
| `POST` | `/api/documents/{id}/sanitize` | Scrub `/Info` dictionary and `/Metadata` XMP streams to prevent information leaks. |
| `GET` | `/api/audit` | List caller action events (upload, redact, sign, optimize) omitting secrets and file bytes. |
| `GET` | `/api/documents/{id}/export` | Download finalized modified PDF with bit-for-bit preserved vector graphics. |
| `WS` | `/ws/documents/{id}/pages/{p}/reflow?ticket=` | Real-time layout reflow. The ticket is consumed on connect. |

---

## Interactive Web Studio (Next.js 16 + React 19)

PDFEngine includes a modern, high-precision web studio inside `web/` with a dual-layer canvas architecture:

* **Dual-Layer Canvas Viewport**: Renders the document canvas with accurate page points and overlays interactive paragraph bounding boxes.
* **Layer 1.5 Image Overlays & Replacement**: Visual inspection of XObject images with floating action buttons for instant in-place PNG/JPEG swapping.
* **Layer 1.8 Interactive AcroForms & Flattening**: In-situ filling for text inputs, checkboxes, and select dropdowns, coupled with single-click surgical document flattening.
* **Layer 1.9 Visual Annotations & Interactive Links**: Real-time rendering of highlights, underlines, strikeouts, clickable links, and rotated rubber stamps.
* **Layer 1.10 Glyph excision and PII scan**: Removes intersecting glyphs from the page content stream, draws a blackout, and can prune intersecting annotations. Document `/Info` and catalog XMP are removed only when scrubbing is requested. Attachments, the structure tree, and form appearances stay. This is not an ISO legal redaction.
* **Document Assembly & Orientation Inspector**: Rotate pages (-90°, +90°, 180°), split documents into single-page chunks, merge external PDFs, and delete pages with live canvas viewport synchronization.
* **Dynamic Foliado & Bates Numbering**: Configurable headers and footers with `{page}` and `{total}` template evaluation, 6-way spatial placement, and cover page bypass.
* **Semi-transparent Text & Image Watermarks**: Rotated diagonal text watermarks and company logos with opacity controls and foreground/background depth placement.
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
- [x] **Phase 10: Document Assembly, Splitting, Merging & Page Operations** (Object cloner, rotation, split, merge, reorder, delete, PyO3 bindings, FastAPI endpoints & Web Studio UI)
- [x] **Phase 11: Annotations, Interactive Links & Vector Rubber Stamps** (Markup annotations, clickable web URIs, internal GoTo navigation, vector rubber stamps with rubrics, surgical flattening)
- [x] **Phase 12: Dynamic Pagination, Bates Numbering & Semitransparent Watermarks** (Headers & footers with `{page}` / `{total}`, Bates numbering, rotated text watermarks, PNG/JPEG logo watermarks, background/foreground depth)
- [x] **Phase 13: Glyph excision, PII scan, and optional metadata scrub** (Physical glyph and stream excision, zero layout shift, PII regex scanning [Email, Phone, RFC, CURP, Credit Card Luhn, SSN], opaque blackout patches, annotation pruning, `/Info` and XMP scrub only when requested, PyO3 bindings, FastAPI endpoints and Web Studio)
- [x] **Phase 14: PDF Security, Permissions & Digital Signatures (ISO 32000 §7.6 & §12.8)** (Standard Security Handler Rev 4 AES-128, permissions bitmask stored in `/P` and not enforced by this process, SHA-256 /ByteRange integrity attestation)
- [x] **Phase 15: Structured Table Reconstruction & Semantic Extraction (ISO 32000 §14.8.4)** (Vector lattice grid solver, borderless fallback, multi-format CSV/JSON/MD/HTML exporters)
- [x] **Phase 16: Lossless PDF Optimization & Stream Compression (ISO 32000-1 §7.5.7)** (Object streams `/ObjStm`, Flate recompression, stream deduplication, unused object pruning)
- [x] **Phase 17: Multi-Page Document Support in Web Studio** (Thumbnail sidebar carousel, visual page reordering, per-page rotation)
- [x] **Phase 18: Interactive AcroForm Builder & Form Field Designer (ISO 32000-1 §12.7)** (AcroForm catalog auto-initialization, merged Widget annotations, visual field designer for Text, Checkbox, Choice, and Digital Signature `/Sig`, field deletion and geometry updating, PyO3 bindings, FastAPI CRUD endpoints & Web Studio)
- [x] **Phase 19: Security Hardening & Vulnerability Remediation** (Bearer identity binding, session TTL & LRU eviction, dynamic SHA-256 byte-range attestation, random AESV2 initialization vectors, active code /JS/Launch action pruning, upload caps, sparse xref streams & cycle guards, predictor & PNG IDAT bounded decompression, safe download headers, table lattice segment budgets, and studio security headers)

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
