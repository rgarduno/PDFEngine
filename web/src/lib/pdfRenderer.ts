'use client';

import type { PDFDocumentProxy, PDFPageProxy, RenderTask } from 'pdfjs-dist';

let pdfjsPromise: Promise<typeof import('pdfjs-dist')> | null = null;

export async function getPdfjs(): Promise<typeof import('pdfjs-dist')> {
  if (typeof window === 'undefined') {
    throw new Error('PDF.js is only available in browser environments');
  }
  if (!pdfjsPromise) {
    pdfjsPromise = import('pdfjs-dist').then((pdfjs) => {
      if (!pdfjs.GlobalWorkerOptions.workerSrc) {
        pdfjs.GlobalWorkerOptions.workerSrc = '/pdf.worker.min.mjs';
      }
      return pdfjs;
    });
  }
  return pdfjsPromise;
}

export interface RenderPdfPageOptions {
  canvas: HTMLCanvasElement;
  pdfData: ArrayBuffer | Uint8Array;
  pageNumber: number;
  zoom: number;
  rotation?: number;
  targetWidthPts?: number;
  targetHeightPts?: number;
}

export class PdfPageRenderer {
  private currentDoc: PDFDocumentProxy | null = null;
  private currentData: ArrayBuffer | Uint8Array | null = null;
  private activeRenderTask: RenderTask | null = null;

  async loadDocument(data: ArrayBuffer | Uint8Array): Promise<PDFDocumentProxy> {
    if (this.currentDoc && this.currentData === data) {
      return this.currentDoc;
    }
    if (this.currentDoc) {
      try {
        await this.currentDoc.cleanup();
        await this.currentDoc.loadingTask.destroy();
      } catch {
        // ignore cleanup error
      }
      this.currentDoc = null;
    }

    const pdfjs = await getPdfjs();
    const loadingTask = pdfjs.getDocument({
      data,
      cMapUrl: '/cmaps/',
      cMapPacked: true,
      standardFontDataUrl: '/standard_fonts/',
    });
    this.currentDoc = await loadingTask.promise;
    this.currentData = data;
    return this.currentDoc;
  }

  async renderPage({
    canvas,
    pdfData,
    pageNumber,
    zoom,
    rotation = 0,
    targetWidthPts = 612,
    targetHeightPts = 792,
  }: RenderPdfPageOptions): Promise<void> {
    if (this.activeRenderTask) {
      try {
        this.activeRenderTask.cancel();
      } catch {
        // ignore cancel error
      }
      this.activeRenderTask = null;
    }

    const doc = await this.loadDocument(pdfData);
    if (pageNumber < 1 || pageNumber > doc.numPages) {
      return;
    }

    const page: PDFPageProxy = await doc.getPage(pageNumber);

    const pixelRatio = typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1;
    
    // Calculate unscaled viewport
    const baseViewport = page.getViewport({ scale: 1.0, rotation: 0 });
    
    // Scale factor to map PDF page points to our target US Letter or native size
    const scaleFactor = Math.min(
      targetWidthPts / (baseViewport.width || targetWidthPts),
      targetHeightPts / (baseViewport.height || targetHeightPts)
    );
    
    const viewport = page.getViewport({
      scale: scaleFactor * zoom * pixelRatio,
      rotation: 0,
    });

    canvas.width = Math.floor(viewport.width);
    canvas.height = Math.floor(viewport.height);
    canvas.style.width = `${Math.floor(viewport.width / pixelRatio)}px`;
    canvas.style.height = `${Math.floor(viewport.height / pixelRatio)}px`;

    const ctx = canvas.getContext('2d', { alpha: false });
    if (!ctx) return;

    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    const renderContext = {
      canvasContext: ctx,
      canvas,
      viewport,
    };

    const task = page.render(renderContext);
    this.activeRenderTask = task;

    try {
      await task.promise;
    } catch (err: unknown) {
      const isCancel = err && typeof err === 'object' && 'name' in err && (err as { name: string }).name === 'RenderingCancelledException';
      if (!isCancel) {
        console.error('PDF.js render error:', err);
      }
    } finally {
      if (this.activeRenderTask === task) {
        this.activeRenderTask = null;
      }
    }
  }

  destroy() {
    if (this.activeRenderTask) {
      try {
        this.activeRenderTask.cancel();
      } catch {
        // ignore cancellation failure
      }
      this.activeRenderTask = null;
    }
    if (this.currentDoc) {
      try {
        this.currentDoc.cleanup();
        this.currentDoc.loadingTask.destroy();
      } catch {
        // ignore destroy failure
      }
      this.currentDoc = null;
    }
    this.currentData = null;
  }
}
