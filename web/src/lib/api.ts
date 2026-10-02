import { DocumentSession, PageSceneGraph, Paragraph, ReflowWebSocketMessage } from './types';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8000';
const WS_BASE_URL = API_BASE_URL.replace(/^http/, 'ws');

// Mock data for immediate preview when backend is not connected
export const MOCK_SESSION: DocumentSession = {
  document_id: 'demo-contract-uuid-001',
  filename: 'commercial_services_agreement.pdf',
  page_count: 1,
};

export const MOCK_SCENEGRAPH: PageSceneGraph = {
  page_number: 1,
  paragraphs: [
    {
      id: 0,
      text: 'MASTER SERVICES AGREEMENT AND COMMERCIAL SPECIFICATION',
      bbox: { min_x: 72, min_y: 710, max_x: 540, max_y: 735, width: 468, height: 25 },
      alignment: 'center',
      leading: 20,
      line_count: 1,
      fontSize: 16,
      fontFamily: 'Helvetica-Bold',
    },
    {
      id: 1,
      text: 'This Master Services Agreement is executed between Enterprise Solutions Inc. ("Client") and PDFEngine Core Technologies LLC ("Provider"), effective as of the execution date set forth below. Parties agree to the terms herein.',
      bbox: { min_x: 72, min_y: 640, max_x: 540, max_y: 695, width: 468, height: 55 },
      alignment: 'left',
      leading: 16,
      line_count: 3,
      fontSize: 11,
      fontFamily: 'Helvetica',
    },
    {
      id: 2,
      text: '1. INTELLECTUAL PROPERTY & SURGICAL IN-PLACE EDITING RIGHTS\nAll underlying Rust parsing modules, ISO 32000 Carousel Object System components, and layout reconstruction engines shall remain the exclusive proprietary technology of Provider.',
      bbox: { min_x: 72, min_y: 560, max_x: 540, max_y: 625, width: 468, height: 65 },
      alignment: 'left',
      leading: 15,
      line_count: 4,
      fontSize: 10,
      fontFamily: 'Helvetica',
    },
    {
      id: 3,
      text: '2. WARRANTY & PERFORMANCE COMMITMENTS\nProvider represents that all modified PDF binary exports will preserve 100% of surrounding vector graphics, clipping boundaries, CMYK color profiles, and font subsets with zero layout drift.',
      bbox: { min_x: 72, min_y: 470, max_x: 540, max_y: 545, width: 468, height: 75 },
      alignment: 'left',
      leading: 15,
      line_count: 4,
      fontSize: 10,
      fontFamily: 'Helvetica',
    },
    {
      id: 4,
      text: 'Approved and Agreed by Authorized Signatories:\n\n__________________________________          __________________________________\nClient Representative                       Provider Executive Officer',
      bbox: { min_x: 72, min_y: 350, max_x: 540, max_y: 430, width: 468, height: 80 },
      alignment: 'left',
      leading: 16,
      line_count: 4,
      fontSize: 10,
      fontFamily: 'Helvetica',
    },
  ],
};

export async function uploadPdf(file: File): Promise<DocumentSession> {
  const formData = new FormData();
  formData.append('file', file);

  try {
    const res = await fetch(`${API_BASE_URL}/api/documents/upload`, {
      method: 'POST',
      body: formData,
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: 'Upload failed' }));
      throw new Error(err.detail || 'Upload failed');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, fallback to mock session:', e);
    return {
      document_id: 'local-' + Date.now(),
      filename: file.name,
      page_count: 1,
    };
  }
}

export async function getPageScenegraph(
  docId: string,
  pageIdx: number
): Promise<PageSceneGraph> {
  try {
    const res = await fetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/scenegraph`
    );
    if (!res.ok) {
      throw new Error(`Failed to load scenegraph: ${res.statusText}`);
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, using mock scenegraph:', e);
    return MOCK_SCENEGRAPH;
  }
}

export async function editParagraph(
  docId: string,
  pageIdx: number,
  paragraphId: number,
  newText: string
): Promise<{ success: boolean; updated_text: string }> {
  try {
    const res = await fetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/edit/${paragraphId}`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ new_text: newText }),
      }
    );
    if (!res.ok) {
      throw new Error('Surgical edit failed');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, applying edit locally:', e);
    return { success: true, updated_text: newText };
  }
}

export function getExportUrl(docId: string): string {
  return `${API_BASE_URL}/api/documents/${docId}/export`;
}

export function connectReflowWebSocket(
  docId: string,
  pageIdx: number,
  onMessage: (msg: ReflowWebSocketMessage) => void
): WebSocket | null {
  try {
    const ws = new WebSocket(
      `${WS_BASE_URL}/ws/documents/${docId}/pages/${pageIdx}/reflow`
    );
    ws.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        onMessage(data);
      } catch (err) {
        console.error('Failed to parse WS reflow payload', err);
      }
    };
    return ws;
  } catch (e) {
    console.warn('WebSocket connection not available in local offline mode', e);
    return null;
  }
}
