# PDFEngine Web Studio

High-performance, dual-layer interactive PDF editing frontend built with **Next.js 16 (React 19)** and **Tailwind CSS**.

Provides in-place surgical text replacement and real-time typographic reflow synchronization connected to the PDFEngine FastAPI microservice and Rust core.

---

## Key Features

* **Dual-Layer Canvas Architecture**:
  * **Layer 1 (Vector Base)**: High-fidelity document canvas rendering exact page bounds (US Letter / A4 in PDF points: `612 x 792 pt`).
  * **Layer 2 (Interactive Overlay)**: Precision-projected paragraph bounding boxes extracted from the ISO 32000 SceneGraph.
* **In-Situ Typographic Editing**:
  * Direct double-click editing on any text block.
  * Real-time baseline positioning, leading, and alignment (`left`, `center`, `right`, `justified`).
* **Live WebSocket Reflow**:
  * Real-time bidirectional streaming over `/ws/documents/{id}/pages/{page}/reflow`.
  * Computes line breaks and bounding box expansions with zero visual latency.
* **Non-Destructive Session History**:
  * Full Undo / Redo history stack with keyboard shortcuts (`Cmd+Z` / `Cmd+Shift+Z`).
* **Lossless Export Pipeline**:
  * One-click download compiling modifications directly through the Rust serialization engine.

---

## Getting Started

### 1. Install Dependencies

```bash
npm install
```

### 2. Configure Backend Endpoint (Optional)

Create `.env.local` to point to the PDFEngine backend (defaults to `http://localhost:8000`):

```bash
NEXT_PUBLIC_API_URL=http://localhost:8000
```

*Note: If the backend is not running, the web studio runs in demonstration mode with interactive mock contracts.*

### 3. Run Development Server

```bash
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

### 4. Build for Production

```bash
npm run build
npm start
```
