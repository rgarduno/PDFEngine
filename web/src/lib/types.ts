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
  annotations?: AnnotationElement[];
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

export interface RotatePageResponse {
  success: boolean;
  document_id: string;
  page_number: number;
  new_rotation: number;
}

export interface SplitDocumentResponse {
  success: boolean;
  source_document_id: string;
  extracted_document_ids: string[];
  count: number;
}

export interface MergeDocumentsResponse {
  success: boolean;
  merged_document_id: string;
  filename: string;
  page_count: number;
}

export type AnnotationSubtype =
  | 'Highlight'
  | 'Underline'
  | 'StrikeOut'
  | 'Link'
  | 'Stamp'
  | 'Other';

export interface AnnotationElement {
  id: number;
  page_index: number;
  page_number: number;
  subtype: AnnotationSubtype;
  bbox: BoundingBox;
  color?: number[];
  opacity: number;
  contents?: string;
  link_type?: 'URI' | 'GoTo';
  link_uri?: string;
  link_target_page?: number;
  stamp_type?: string;
  date_str?: string;
}

export interface PageAnnotationsResponse {
  document_id: string;
  page_number: number;
  count: number;
  annotations: AnnotationElement[];
}

export interface AddMarkupPayload {
  subtype: 'Highlight' | 'Underline' | 'StrikeOut';
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
  color?: number[];
  opacity?: number;
  contents?: string;
}

export interface AddLinkPayload {
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
  uri?: string;
  target_page?: number;
  show_border?: boolean;
}

export interface AddStampPayload {
  stamp_type: string;
  min_x?: number;
  min_y?: number;
  max_x?: number;
  max_y?: number;
  custom_text?: string;
  color?: number[];
  date_str?: string;
}

export interface AnnotationActionResponse {
  success: boolean;
  document_id: string;
  page_number: number;
  annotation_id: number;
  message: string;
}

export interface FlattenAnnotationsResponse {
  success: boolean;
  document_id: string;
  flattened_count: number;
  message: string;
}
