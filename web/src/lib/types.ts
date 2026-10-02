export interface BoundingBox {
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
  width: number;
  height: number;
}

export type TextAlignment = 'left' | 'center' | 'right' | 'justified';

export interface Paragraph {
  id: number;
  text: string;
  bbox: BoundingBox;
  alignment: TextAlignment;
  leading: number;
  line_count: number;
  fontSize?: number;
  fontFamily?: string;
}

export interface ImageElement {
  id: number;
  name: string;
  width_px: number;
  height_px: number;
  color_space: string;
  bits_per_component: number;
  filter?: string;
  byte_size: number;
  bbox: BoundingBox;
}

export interface PageSceneGraph {
  page_number: number;
  paragraphs: Paragraph[];
  images?: ImageElement[];
}

export interface DocumentSession {
  document_id: string;
  filename: string;
  page_count: number;
}

export interface ReflowWebSocketMessage {
  status: 'ok' | 'error';
  paragraph_id: number;
  line_count?: number;
  text?: string;
  bbox?: BoundingBox;
  message?: string;
}

export interface HistoryEntry {
  paragraphs: Paragraph[];
  description: string;
  timestamp: number;
}
