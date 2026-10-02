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
  forms?: FormFieldElement[];
}

export type FormFieldType =
  | 'Text'
  | 'Checkbox'
  | 'RadioButton'
  | 'PushButton'
  | 'Choice'
  | 'Signature';

export interface FormFieldElement {
  id: number;
  name: string;
  alt_name?: string;
  field_type: FormFieldType;
  value: string;
  default_value?: string;
  bbox: BoundingBox;
  page_number: number;
  options: string[];
  is_read_only: boolean;
  is_required: boolean;
  is_multiline: boolean;
  max_length?: number;
}

export interface DocumentFormsResponse {
  document_id: string;
  count: number;
  fields: FormFieldElement[];
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
