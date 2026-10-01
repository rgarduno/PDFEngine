# PDFEngine

High-performance, memory-safe, lossless PDF parsing and surgical editing engine built in Rust with Python and WebAssembly bindings.

Designed for high-throughput cloud environments, serverless functions (AWS Lambda, Google Cloud Functions), containerized microservices, and interactive web applications.

---

## Overview

Most existing PDF editing tools rely on lossy document conversion (e.g., converting to HTML, DOCX, or raster images) or naive string replacement, which frequently destroys vector drawings, clipping paths, embedded fonts, and paragraph layouts.

**PDFEngine** provides an ISO 32000-compliant, deterministic, and safe engine that performs **surgical in-place mutations**:
* **Lossless AST Representation**: Preserves 100% of non-edited vector paths, blend modes, images, color spaces, and graphics states (`q ... Q`).
* **Semantic Layout Reconstruction**: Clusters glyph runs into spans, text lines, and paragraph blocks with automatic baseline and alignment detection.
* **Typographic-Safe Reflow**: Recalculates line breaks and horizontal advances using embedded font metrics (`/Widths`, `cmap`, `hmtx`), handling font subsetting, ligature decomposition, and ghost space inference.
* **Memory Safety & Hardening**: Implemented in safe Rust to eliminate buffer overflows, use-after-free conditions, circular reference loops, and decompression bomb vectors (`FlateDecode` expansion caps).

---

## Architecture

```
PDFEngine/
├── crates/
│   ├── pdf-engine-core/        # Pure Rust core (COS parser, AST, layout, editor, serializer)
│   └── pdf-engine-ffi/         # C-ABI and WebAssembly bindings
├── bindings/
│   └── python/                 # PyO3 high-level Python bindings
└── tests/
    └── fixtures/               # Real-world conformance and regression test corpus
```

---

## Features

- **ISO 32000-1 / 32000-2 Conformance**: Robust parsing of indirect objects, classic XRef tables, hybrid reference streams, and object streams (`/ObjStm`).
- **Defensive Decompression**: Bounded zlib expansion ratios (max 100:1, default 250 MB ceiling) preventing resource exhaustion attacks.
- **Surgical Content Stream Mutator**: Atomic replacement of target `BT ... ET` text operator sequences without mutating surrounding graphic streams.
- **Zero-Copy Parser Primitives**: High-performance byte slice scanning using zero-copy tokenization where applicable.
- **Multi-Platform Targeting**: Compiles natively for Linux (`x86_64`, `aarch64`), macOS, Windows, and `wasm32-unknown-unknown`.

---

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
