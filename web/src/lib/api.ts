import {
  DocumentFormsResponse,
  DocumentSession,
  FormFieldElement,
  ImageElement,
  PageSceneGraph,
  Paragraph,
  ReflowWebSocketMessage,
} from './types';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8000';
const WS_BASE_URL = API_BASE_URL.replace(/^http/, 'ws');

// Mock data for immediate preview when backend is not connected
export const MOCK_SESSION: DocumentSession = {
  document_id: 'demo-contract-uuid-001',
  filename: 'commercial_services_agreement.pdf',
  page_count: 1,
};

export const MOCK_IMAGES: ImageElement[] = [
  {
    id: 101,
    name: 'ImLogo',
    width_px: 240,
    height_px: 60,
    color_space: 'DeviceRGB',
    bits_per_component: 8,
    filter: 'DCTDecode',
    byte_size: 4820,
    bbox: { min_x: 72, min_y: 745, max_x: 212, max_y: 780, width: 140, height: 35 },
  },
];

export const MOCK_FORMS: FormFieldElement[] = [
  {
    id: 201,
    name: 'AuthorizedSignatory',
    alt_name: 'Full Name of Signatory',
    field_type: 'Text',
    value: 'Johnathan Smith',
    bbox: { min_x: 72, min_y: 310, max_x: 260, max_y: 335, width: 188, height: 25 },
    page_number: 1,
    options: [],
    is_read_only: false,
    is_required: true,
    is_multiline: false,
  },
  {
    id: 202,
    name: 'ContractTerm',
    alt_name: 'Subscription Period',
    field_type: 'Choice',
    value: 'Annual Enterprise License',
    bbox: { min_x: 320, min_y: 310, max_x: 540, max_y: 335, width: 220, height: 25 },
    page_number: 1,
    options: ['Monthly Standard', 'Annual Enterprise License', 'Multi-Year Dedicated'],
    is_read_only: false,
    is_required: true,
    is_multiline: false,
  },
  {
    id: 203,
    name: 'AgreeToTerms',
    alt_name: 'Accept Terms and Conditions',
    field_type: 'Checkbox',
    value: 'Yes',
    bbox: { min_x: 72, min_y: 275, max_x: 92, max_y: 295, width: 20, height: 20 },
    page_number: 1,
    options: [],
    is_read_only: false,
    is_required: true,
    is_multiline: false,
  },
];

export const MOCK_SCENEGRAPH: PageSceneGraph = {
  page_number: 1,
  images: MOCK_IMAGES,
  forms: MOCK_FORMS,
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

export async function getPageFonts(
  docId: string,
  pageIdx: number
): Promise<{ page_number: number; fonts: string[]; embedded_count: number }> {
  try {
    const res = await fetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/fonts`
    );
    if (!res.ok) {
      return { page_number: pageIdx, fonts: [], embedded_count: 0 };
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, font extraction fallback:', e);
    return { page_number: pageIdx, fonts: [], embedded_count: 0 };
  }
}

export function getFontBinaryUrl(
  docId: string,
  pageIdx: number,
  fontName: string
): string {
  return `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/fonts/${encodeURIComponent(fontName)}`;
}

export async function getPageImages(
  docId: string,
  pageIdx: number
): Promise<{ page_number: number; images: ImageElement[]; count: number }> {
  try {
    const res = await fetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/images`
    );
    if (!res.ok) {
      return { page_number: pageIdx, images: MOCK_IMAGES, count: MOCK_IMAGES.length };
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, using mock image data:', e);
    return { page_number: pageIdx, images: MOCK_IMAGES, count: MOCK_IMAGES.length };
  }
}

export function getImageBinaryUrl(docId: string, imageId: number): string {
  return `${API_BASE_URL}/api/documents/${docId}/images/${imageId}`;
}

export async function replaceImage(
  docId: string,
  imageId: number,
  file: File
): Promise<{ success: boolean; message: string }> {
  const formData = new FormData();
  formData.append('file', file);

  try {
    const res = await fetch(
      `${API_BASE_URL}/api/documents/${docId}/images/${imageId}/replace`,
      {
        method: 'POST',
        body: formData,
      }
    );
    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: 'Image replacement failed' }));
      throw new Error(err.detail || 'Image replacement failed');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock image replace:', e);
    return { success: true, message: `Image ${imageId} updated locally.` };
  }
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

export async function getDocumentForms(
  docId: string
): Promise<DocumentFormsResponse> {
  try {
    const res = await fetch(`${API_BASE_URL}/api/documents/${docId}/forms`);
    if (!res.ok) {
      return { document_id: docId, count: MOCK_FORMS.length, fields: MOCK_FORMS };
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, using mock form data:', e);
    return { document_id: docId, count: MOCK_FORMS.length, fields: MOCK_FORMS };
  }
}

export async function fillFormField(
  docId: string,
  fieldName: string,
  value: string
): Promise<{ success: boolean; updated_count: number }> {
  try {
    const res = await fetch(`${API_BASE_URL}/api/documents/${docId}/forms/fill`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ fields: { [fieldName]: value } }),
    });
    if (!res.ok) {
      throw new Error('Failed to fill form field');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock form fill:', e);
    return { success: true, updated_count: 1 };
  }
}

export async function flattenDocumentForms(
  docId: string
): Promise<{ success: boolean; flattened_count: number; message: string }> {
  try {
    const res = await fetch(
      `${API_BASE_URL}/api/documents/${docId}/forms/flatten`,
      { method: 'POST' }
    );
    if (!res.ok) {
      throw new Error('Failed to flatten form fields');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock form flatten:', e);
    return {
      success: true,
      flattened_count: MOCK_FORMS.length,
      message: 'All form fields flattened locally.',
    };
  }
}
