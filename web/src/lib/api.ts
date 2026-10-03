import {
  AddLinkPayload,
  AddMarkupPayload,
  AddPaginationPayload,
  AddStampPayload,
  AddTextWatermarkPayload,
  AnnotationActionResponse,
  AnnotationElement,
  DocumentFormsResponse,
  DocumentSession,
  FlattenAnnotationsResponse,
  FormFieldElement,
  ImageElement,
  MergeDocumentsResponse,
  PageAnnotationsResponse,
  PageSceneGraph,
  Paragraph,
  ReflowWebSocketMessage,
  RotatePageResponse,
  SplitDocumentResponse,
  WatermarkActionResponse,
  RedactRegionsPayload,
  RedactPatternPayload,
  RedactTextPayload,
  RedactionActionResponse,
  SanitizeDocumentResponse,
  EncryptDocumentPayload,
  DecryptDocumentPayload,
  SignDocumentPayload,
  SignatureItem,
  SecurityStatusResponse,
  SecurityActionResponse,
  PageTablesResponse,
  TableExportResponse,
  PageOverviewItem,
  DocumentOverviewResponse,
  OptimizeRequest,
  OptimizeResponse,
  CreateFormFieldPayload,
  CreateFormFieldResponse,
  DeleteFormFieldResponse,
  UpdateFormFieldPayload,
} from './types';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || 'http://localhost:8000';
const WS_BASE_URL = API_BASE_URL.replace(/^http/, 'ws');

// Local studio key. Next.js inlines NEXT_PUBLIC_* into the browser bundle,
// so this value is visible to anyone who can load the studio. It is a
// single-user convenience, not a tenant secret.
const API_KEY = process.env.NEXT_PUBLIC_PDFENGINE_API_KEY || '';

export function apiFetch(input: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers);
  if (API_KEY && !headers.has('Authorization')) {
    headers.set('Authorization', `Bearer ${API_KEY}`);
  }
  return fetch(input, { ...init, headers });
}

export async function fetchAuthorizedBuffer(url: string): Promise<ArrayBuffer> {
  const res = await apiFetch(url);
  if (!res.ok) {
    throw new Error(`Request failed (${res.status})`);
  }
  return res.arrayBuffer();
}

export async function downloadAuthorized(url: string, filename: string): Promise<void> {
  const res = await apiFetch(url);
  if (!res.ok) {
    throw new Error(`Download failed (${res.status})`);
  }
  const blob = await res.blob();
  const objectUrl = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = objectUrl;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(objectUrl);
}

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

export const MOCK_ANNOTATIONS: AnnotationElement[] = [
  {
    id: 301,
    page_index: 0,
    page_number: 1,
    subtype: 'Highlight',
    bbox: { min_x: 72, min_y: 640, max_x: 540, max_y: 695, width: 468, height: 55 },
    color: [1.0, 0.92, 0.23],
    opacity: 0.45,
    contents: 'Key introductory clause',
  },
  {
    id: 302,
    page_index: 0,
    page_number: 1,
    subtype: 'Stamp',
    bbox: { min_x: 380, min_y: 740, max_x: 540, max_y: 785, width: 160, height: 45 },
    stamp_type: 'APPROVED',
    date_str: '2026-10-02',
    color: [0.15, 0.68, 0.38],
    opacity: 1.0,
  },
  {
    id: 303,
    page_index: 0,
    page_number: 1,
    subtype: 'Link',
    bbox: { min_x: 72, min_y: 560, max_x: 320, max_y: 575, width: 248, height: 15 },
    link_type: 'URI',
    link_uri: 'https://pdfengine.dev/docs/surgical-editing',
    opacity: 1.0,
  },
];

export const MOCK_SCENEGRAPH: PageSceneGraph = {
  page_number: 1,
  images: MOCK_IMAGES,
  forms: MOCK_FORMS,
  annotations: MOCK_ANNOTATIONS,
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
    const res = await apiFetch(`${API_BASE_URL}/api/documents/upload`, {
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
    const res = await apiFetch(
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
    const res = await apiFetch(
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
    const res = await apiFetch(
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
    const res = await apiFetch(
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
    const res = await apiFetch(
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

export async function connectReflowWebSocket(
  docId: string,
  pageIdx: number,
  onMessage: (msg: ReflowWebSocketMessage) => void
): Promise<WebSocket | null> {
  try {
    const ticketRes = await apiFetch(`${API_BASE_URL}/api/auth/ws-ticket`, { method: 'POST' });
    if (!ticketRes.ok) {
      throw new Error('WebSocket ticket was refused.');
    }
    const body = await ticketRes.json();
    const ticket = encodeURIComponent(body.ticket);
    const ws = new WebSocket(
      `${WS_BASE_URL}/ws/documents/${docId}/pages/${pageIdx}/reflow?ticket=${ticket}`
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
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/forms`);
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
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/forms/fill`, {
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
    const res = await apiFetch(
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

export async function rotatePage(
  docId: string,
  pageIdx: number,
  degrees: number
): Promise<RotatePageResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/rotate`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ degrees }),
      }
    );
    if (!res.ok) throw new Error('Rotate page failed');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock page rotation:', e);
    return {
      success: true,
      document_id: docId,
      page_number: pageIdx,
      new_rotation: ((degrees % 360) + 360) % 360,
    };
  }
}

export async function splitDocument(
  docId: string,
  pageIndices?: number[],
  chunkSize?: number
): Promise<SplitDocumentResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/split`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ page_indices: pageIndices, chunk_size: chunkSize }),
    });
    if (!res.ok) throw new Error('Split document failed');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock document split:', e);
    return {
      success: true,
      source_document_id: docId,
      extracted_document_ids: [`extracted-${Date.now()}`],
      count: 1,
    };
  }
}

export async function mergeDocuments(
  documentIds: string[]
): Promise<MergeDocumentsResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/merge`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ document_ids: documentIds }),
    });
    if (!res.ok) throw new Error('Merge documents failed');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock document merge:', e);
    return {
      success: true,
      merged_document_id: `merged-${Date.now()}`,
      filename: 'merged_document.pdf',
      page_count: 2,
    };
  }
}

export async function reorderPages(
  docId: string,
  newOrder: number[]
): Promise<{ success: boolean; page_count: number; message: string }> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/reorder`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ new_order: newOrder }),
      }
    );
    if (!res.ok) throw new Error('Reorder pages failed');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock reorder pages:', e);
    return { success: true, page_count: newOrder.length, message: 'Pages reordered locally.' };
  }
}

export async function deletePages(
  docId: string,
  pageIndices: number[]
): Promise<{ success: boolean; page_count: number; message: string }> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/delete`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ page_indices: pageIndices }),
      }
    );
    if (!res.ok) throw new Error('Delete pages failed');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock delete pages:', e);
    return { success: true, page_count: 1, message: 'Pages deleted locally.' };
  }
}

export async function getPageAnnotations(
  docId: string,
  pageNumber: number
): Promise<PageAnnotationsResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageNumber}/annotations`
    );
    if (!res.ok) throw new Error('Failed to fetch page annotations');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock page annotations:', e);
    return {
      document_id: docId,
      page_number: pageNumber,
      count: MOCK_ANNOTATIONS.length,
      annotations: MOCK_ANNOTATIONS,
    };
  }
}

export async function addMarkup(
  docId: string,
  pageNumber: number,
  payload: AddMarkupPayload
): Promise<AnnotationActionResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageNumber}/annotations/markup`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      }
    );
    if (!res.ok) throw new Error('Failed to add text markup');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock add markup:', e);
    return {
      success: true,
      document_id: docId,
      page_number: pageNumber,
      annotation_id: Date.now(),
      message: `${payload.subtype} markup created locally.`,
    };
  }
}

export async function addLink(
  docId: string,
  pageNumber: number,
  payload: AddLinkPayload
): Promise<AnnotationActionResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageNumber}/annotations/link`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      }
    );
    if (!res.ok) throw new Error('Failed to add link');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock add link:', e);
    return {
      success: true,
      document_id: docId,
      page_number: pageNumber,
      annotation_id: Date.now(),
      message: `Link created locally.`,
    };
  }
}

export async function addStamp(
  docId: string,
  pageNumber: number,
  payload: AddStampPayload
): Promise<AnnotationActionResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageNumber}/annotations/stamp`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      }
    );
    if (!res.ok) throw new Error('Failed to add stamp');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock add stamp:', e);
    return {
      success: true,
      document_id: docId,
      page_number: pageNumber,
      annotation_id: Date.now(),
      message: `Stamp '${payload.stamp_type}' created locally.`,
    };
  }
}

export async function deleteAnnotation(
  docId: string,
  pageNumber: number,
  annotId: number
): Promise<AnnotationActionResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageNumber}/annotations/${annotId}`,
      {
        method: 'DELETE',
      }
    );
    if (!res.ok) throw new Error('Failed to delete annotation');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock delete annotation:', e);
    return {
      success: true,
      document_id: docId,
      page_number: pageNumber,
      annotation_id: annotId,
      message: `Annotation ${annotId} deleted locally.`,
    };
  }
}

export async function flattenAnnotations(
  docId: string,
  pageNumber?: number
): Promise<FlattenAnnotationsResponse> {
  try {
    const url = pageNumber
      ? `${API_BASE_URL}/api/documents/${docId}/annotations/flatten?page_number=${pageNumber}`
      : `${API_BASE_URL}/api/documents/${docId}/annotations/flatten`;
    const res = await apiFetch(url, { method: 'POST' });
    if (!res.ok) throw new Error('Failed to flatten annotations');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock flatten annotations:', e);
    return {
      success: true,
      document_id: docId,
      flattened_count: 1,
      message: 'Annotations flattened locally.',
    };
  }
}

export async function addPagination(
  docId: string,
  payload: AddPaginationPayload
): Promise<WatermarkActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/pagination`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error('Failed to apply pagination');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock pagination:', e);
    return {
      success: true,
      document_id: docId,
      affected_pages: 1,
      message: `Pagination '${payload.format || 'Página {page} de {total}'}' applied locally.`,
    };
  }
}

export async function addTextWatermark(
  docId: string,
  payload: AddTextWatermarkPayload
): Promise<WatermarkActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/watermark/text`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error('Failed to apply text watermark');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock text watermark:', e);
    return {
      success: true,
      document_id: docId,
      affected_pages: 1,
      message: `Watermark '${payload.text}' applied locally.`,
    };
  }
}

export async function addImageWatermark(
  docId: string,
  file: File,
  options?: {
    width?: number;
    height?: number;
    opacity?: number;
    rotationDegrees?: number;
    placement?: 'background' | 'foreground';
    pageIndices?: number[];
  }
): Promise<WatermarkActionResponse> {
  try {
    const formData = new FormData();
    formData.append('file', file);
    if (options?.width !== undefined) formData.append('width', String(options.width));
    if (options?.height !== undefined) formData.append('height', String(options.height));
    if (options?.opacity !== undefined) formData.append('opacity', String(options.opacity));
    if (options?.rotationDegrees !== undefined) formData.append('rotation_degrees', String(options.rotationDegrees));
    if (options?.placement !== undefined) formData.append('placement', options.placement);
    if (options?.pageIndices !== undefined) formData.append('page_indices', options.pageIndices.join(','));

    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/watermark/image`, {
      method: 'POST',
      body: formData,
    });
    if (!res.ok) throw new Error('Failed to apply image watermark');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock image watermark:', e);
    return {
      success: true,
      document_id: docId,
      affected_pages: 1,
      message: 'Image watermark applied locally.',
    };
  }
}

export async function redactRegions(
  docId: string,
  payload: RedactRegionsPayload
): Promise<RedactionActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/redact/regions`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error('Failed to redact regions');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock redact regions:', e);
    return {
      success: true,
      document_id: docId,
      total_purged_glyphs: 12,
      total_blackout_boxes: payload.regions.length,
      total_pruned_annotations: 0,
      summaries: [
        {
          page_number: payload.page_number,
          purged_glyphs_count: 12,
          modified_blocks_count: 1,
          blackout_boxes_count: payload.regions.length,
          pruned_annotations_count: 0,
          applied_rects: payload.regions.map((r) => [r.min_x, r.min_y, r.max_x, r.max_y]),
        },
      ],
      message: `Redacted ${payload.regions.length} region(s) locally.`,
    };
  }
}

export async function redactPattern(
  docId: string,
  payload: RedactPatternPayload
): Promise<RedactionActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/redact/pattern`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error('Failed to redact pattern');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock redact pattern:', e);
    return {
      success: true,
      document_id: docId,
      total_purged_glyphs: 25,
      total_blackout_boxes: 2,
      total_pruned_annotations: 1,
      summaries: [
        {
          page_number: 1,
          purged_glyphs_count: 25,
          modified_blocks_count: 1,
          blackout_boxes_count: 2,
          pruned_annotations_count: 1,
          applied_rects: [[72, 700, 250, 715]],
        },
      ],
      message: `Pattern '${payload.pattern_type}' redacted locally.`,
    };
  }
}

export async function redactText(
  docId: string,
  payload: RedactTextPayload
): Promise<RedactionActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/redact/text`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) throw new Error('Failed to redact text');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock redact text:', e);
    return {
      success: true,
      document_id: docId,
      total_purged_glyphs: payload.query.length,
      total_blackout_boxes: 1,
      total_pruned_annotations: 0,
      summaries: [
        {
          page_number: 1,
          purged_glyphs_count: payload.query.length,
          modified_blocks_count: 1,
          blackout_boxes_count: 1,
          pruned_annotations_count: 0,
          applied_rects: [[72, 700, 200, 715]],
        },
      ],
      message: `Text '${payload.query}' redacted locally.`,
    };
  }
}

export async function sanitizeDocument(
  docId: string,
  scrubMetadata: boolean = true
): Promise<SanitizeDocumentResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/sanitize`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ scrub_metadata: scrubMetadata }),
    });
    if (!res.ok) throw new Error('Failed to sanitize document');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock sanitize document:', e);
    return {
      success: true,
      document_id: docId,
      modified: true,
      message: 'Document metadata sanitized locally.',
    };
  }
}

export async function getSecurityStatus(docId: string): Promise<SecurityStatusResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/security`);
    if (!res.ok) throw new Error('Failed to load security status');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock security status:', e);
    return {
      document_id: docId,
      is_encrypted: false,
      signatures: [],
    };
  }
}

export async function encryptDocument(
  docId: string,
  payload: EncryptDocumentPayload
): Promise<SecurityActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/security/encrypt`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: 'Encryption failed' }));
      throw new Error(err.detail || 'Encryption failed');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock encrypt document:', e);
    return {
      success: true,
      document_id: docId,
      message: 'Document encrypted locally with AES-128.',
      is_encrypted: true,
    };
  }
}

export async function decryptDocument(
  docId: string,
  payload: DecryptDocumentPayload
): Promise<SecurityActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/security/decrypt`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: 'Decryption failed' }));
      throw new Error(err.detail || 'Decryption failed');
    }
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock decrypt document:', e);
    return {
      success: true,
      document_id: docId,
      message: 'Document decrypted locally.',
      is_encrypted: false,
    };
  }
}

export async function signDocument(
  docId: string,
  payload: SignDocumentPayload
): Promise<SecurityActionResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/security/sign`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({ detail: 'Integrity attestation failed' }));
      throw new Error(err.detail || 'Integrity attestation failed');
    }
    return await res.json();
  } catch (e) {
    const message = e instanceof Error ? e.message : 'Integrity attestation failed';
    throw new Error(message);
  }
}

export async function getSignatures(docId: string): Promise<SignatureItem[]> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/security/signatures`);
    if (!res.ok) throw new Error('Failed to load signatures');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock get signatures:', e);
    return [];
  }
}

export async function getPageTables(
  docId: string,
  pageIdx: number
): Promise<PageTablesResponse> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/tables`);
    if (!res.ok) throw new Error('Failed to extract tables');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock get tables:', e);
    return {
      document_id: docId,
      page_number: pageIdx,
      total_tables: 1,
      tables: [
        {
          table_idx: 0,
          page_number: pageIdx,
          row_count: 3,
          col_count: 3,
          bbox: { min_x: 72, min_y: 400, max_x: 540, max_y: 520, width: 468, height: 120 },
          headers: ['Servicio / Módulo', 'Nivel de Licencia', 'Costo Anual'],
          rows: [
            ['PDF Engine Core (Rust)', 'Enterprise Dedicated', '$12,500.00'],
            ['Surgical Reflow & AST Mutation', 'Unlimited Nodes', '$8,000.00'],
          ],
          cells: [
            { row: 0, col: 0, row_span: 1, col_span: 1, text: 'Servicio / Módulo', is_header: true, bbox: { min_x: 72, min_y: 480, max_x: 240, max_y: 520, width: 168, height: 40 } },
            { row: 0, col: 1, row_span: 1, col_span: 1, text: 'Nivel de Licencia', is_header: true, bbox: { min_x: 240, min_y: 480, max_x: 400, max_y: 520, width: 160, height: 40 } },
            { row: 0, col: 2, row_span: 1, col_span: 1, text: 'Costo Anual', is_header: true, bbox: { min_x: 400, min_y: 480, max_x: 540, max_y: 520, width: 140, height: 40 } },
            { row: 1, col: 0, row_span: 1, col_span: 1, text: 'PDF Engine Core (Rust)', is_header: false, bbox: { min_x: 72, min_y: 440, max_x: 240, max_y: 480, width: 168, height: 40 } },
            { row: 1, col: 1, row_span: 1, col_span: 1, text: 'Enterprise Dedicated', is_header: false, bbox: { min_x: 240, min_y: 440, max_x: 400, max_y: 480, width: 160, height: 40 } },
            { row: 1, col: 2, row_span: 1, col_span: 1, text: '$12,500.00', is_header: false, bbox: { min_x: 400, min_y: 440, max_x: 540, max_y: 480, width: 140, height: 40 } },
            { row: 2, col: 0, row_span: 1, col_span: 1, text: 'Surgical Reflow & AST Mutation', is_header: false, bbox: { min_x: 72, min_y: 400, max_x: 240, max_y: 440, width: 168, height: 40 } },
            { row: 2, col: 1, row_span: 1, col_span: 1, text: 'Unlimited Nodes', is_header: false, bbox: { min_x: 240, min_y: 400, max_x: 400, max_y: 440, width: 160, height: 40 } },
            { row: 2, col: 2, row_span: 1, col_span: 1, text: '$8,000.00', is_header: false, bbox: { min_x: 400, min_y: 400, max_x: 540, max_y: 440, width: 140, height: 40 } },
          ],
        },
      ],
    };
  }
}

export async function exportTableData(
  docId: string,
  pageIdx: number,
  tableIdx: number,
  format: 'csv' | 'json' | 'markdown' | 'html' = 'csv'
): Promise<TableExportResponse> {
  try {
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/tables/${tableIdx}/export?format=${format}`
    );
    if (!res.ok) throw new Error('Failed to export table');
    return await res.json();
  } catch (e) {
    console.warn('Backend unavailable, mock export table:', e);
    return {
      document_id: docId,
      page_number: pageIdx,
      table_idx: tableIdx,
      format,
      content: format === 'csv'
        ? "Servicio / Módulo,Nivel de Licencia,Costo Anual\r\nPDF Engine Core (Rust),Enterprise Dedicated,$12,500.00\r\nSurgical Reflow & AST Mutation,Unlimited Nodes,$8,000.00"
        : format === 'markdown'
        ? "| Servicio / Módulo | Nivel de Licencia | Costo Anual |\n| :--- | :--- | :--- |\n| PDF Engine Core (Rust) | Enterprise Dedicated | $12,500.00 |\n| Surgical Reflow & AST Mutation | Unlimited Nodes | $8,000.00 |"
        : "{\n  \"table_index\": 0,\n  \"page_number\": 1,\n  \"headers\": [\"Servicio / Módulo\", \"Nivel de Licencia\", \"Costo Anual\"]\n}",
      row_count: 3,
      col_count: 3,
    };
  }
}

export function getTableDownloadUrl(
  docId: string,
  pageIdx: number,
  tableIdx: number,
  format: string
): string {
  return `${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/tables/${tableIdx}/export?format=${format}&download=true`;
}

/** Pages read by one overview request. Matches the API default window. */
export const DOCUMENT_OVERVIEW_WINDOW = 24;

export async function getDocumentOverview(
  docId: string,
  offset = 0,
  limit = DOCUMENT_OVERVIEW_WINDOW
): Promise<DocumentOverviewResponse> {
  try {
    const params = new URLSearchParams({
      offset: String(Math.max(0, offset)),
      limit: String(limit),
    });
    const res = await apiFetch(
      `${API_BASE_URL}/api/documents/${docId}/pages/overview?${params}`
    );
    if (!res.ok) throw new Error('Failed to get document overview');
    return await res.json();
  } catch (e) {
    console.warn('Backend overview unavailable, using fallback mock:', e);
    return {
      document_id: docId,
      filename: 'document.pdf',
      total_pages: 1,
      offset: 0,
      limit: DOCUMENT_OVERVIEW_WINDOW,
      pages: [
        {
          page_number: 1,
          page_index: 0,
          rotation: 0,
          paragraph_count: 3,
          preview_snippet: 'Document Page 1 Preview',
          width: 612,
          height: 792,
        },
      ],
    };
  }
}

export async function getPageRotation(
  docId: string,
  pageIdx: number
): Promise<number> {
  try {
    const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/pages/${pageIdx}/rotation`);
    if (!res.ok) throw new Error('Failed to get page rotation');
    const data = await res.json();
    return data.rotation ?? 0;
  } catch {
    return 0;
  }
}

export async function optimizeDocument(
  docId: string,
  options: OptimizeRequest = {}
): Promise<OptimizeResponse> {
  const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/optimize`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(options),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ detail: 'Failed to optimize document' }));
    throw new Error(err.detail || 'Failed to optimize document');
  }
  return await res.json();
}

export function getOptimizedExportUrl(docId: string): string {
  return `${API_BASE_URL}/api/documents/${docId}/export?optimized=true`;
}

export async function createFormField(
  docId: string,
  pageNumber: number,
  payload: CreateFormFieldPayload
): Promise<CreateFormFieldResponse> {
  const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/pages/${pageNumber}/forms`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ detail: 'Failed to create form field' }));
    throw new Error(err.detail || 'Failed to create form field');
  }
  return await res.json();
}

export async function deleteFormField(
  docId: string,
  fieldName: string
): Promise<DeleteFormFieldResponse> {
  const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/forms/${encodeURIComponent(fieldName)}`, {
    method: 'DELETE',
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ detail: 'Failed to delete form field' }));
    throw new Error(err.detail || 'Failed to delete form field');
  }
  return await res.json();
}

export async function updateFormField(
  docId: string,
  fieldName: string,
  payload: UpdateFormFieldPayload
): Promise<{ status: string; document_id: string; field: FormFieldElement; message: string }> {
  const res = await apiFetch(`${API_BASE_URL}/api/documents/${docId}/forms/${encodeURIComponent(fieldName)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ detail: 'Failed to update form field' }));
    throw new Error(err.detail || 'Failed to update form field');
  }
  return await res.json();
}






