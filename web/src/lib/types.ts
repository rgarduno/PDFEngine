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

export interface AddPaginationPayload {
  format?: string;
  position?: 'top_left' | 'top_center' | 'top_right' | 'bottom_left' | 'bottom_center' | 'bottom_right';
  font_size?: number;
  color?: number[];
  margin?: number;
  start_page_num?: number;
  skip_first_page?: boolean;
  page_indices?: number[];
}

export interface AddTextWatermarkPayload {
  text: string;
  font_size?: number;
  color?: number[];
  opacity?: number;
  rotation_degrees?: number;
  placement?: 'background' | 'foreground';
  page_indices?: number[];
}

export interface WatermarkActionResponse {
  success: boolean;
  document_id: string;
  affected_pages: number;
  message: string;
}

export interface RedactionRegionItem {
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
}

export interface RedactRegionsPayload {
  page_number: number;
  regions: RedactionRegionItem[];
  fill_color?: number[];
  overlay_text?: string;
  text_color?: number[];
  font_size?: number;
  prune_annotations?: boolean;
}

export interface RedactPatternPayload {
  pattern_type: 'email' | 'phone' | 'ssn' | 'credit_card' | 'rfc' | 'curp' | 'text';
  custom_query?: string;
  case_sensitive?: boolean;
  page_numbers?: number[];
  fill_color?: number[];
  overlay_text?: string;
  text_color?: number[];
  font_size?: number;
  prune_annotations?: boolean;
  scrub_metadata?: boolean;
}

export interface RedactTextPayload {
  query: string;
  case_sensitive?: boolean;
  page_numbers?: number[];
  fill_color?: number[];
  overlay_text?: string;
  text_color?: number[];
  font_size?: number;
  prune_annotations?: boolean;
}

export interface RedactionSummaryItem {
  page_number: number;
  purged_glyphs_count: number;
  modified_blocks_count: number;
  blackout_boxes_count: number;
  pruned_annotations_count: number;
  applied_rects: number[][];
}

export interface RedactionActionResponse {
  success: boolean;
  document_id: string;
  total_purged_glyphs: number;
  total_blackout_boxes: number;
  total_pruned_annotations: number;
  summaries: RedactionSummaryItem[];
  message: string;
}

export interface SanitizeDocumentResponse {
  success: boolean;
  document_id: string;
  modified: boolean;
  message: string;
}

export interface PermissionsPayload {
  print_low_res?: boolean;
  print_high_res?: boolean;
  modify_contents?: boolean;
  copy_extract?: boolean;
  modify_annotations?: boolean;
  fill_forms?: boolean;
  accessibility_extract?: boolean;
  assemble_document?: boolean;
}

export interface EncryptDocumentPayload {
  user_password?: string;
  owner_password?: string;
  permissions?: PermissionsPayload;
  encrypt_metadata?: boolean;
}

export interface DecryptDocumentPayload {
  password: string;
}

export interface SignDocumentPayload {
  signer_name: string;
  reason: string;
  location: string;
  page_number?: number;
  rect?: number[];
  contact_info?: string;
}

export interface SignatureItem {
  field_name: string;
  signer_name: string;
  reason: string;
  location: string;
  date: string;
  sub_filter: string;
  byte_range: number[];
  contents_hex: string;
  byte_range_valid: boolean;
  rect: number[];
  page_number: number;
}

export interface SecurityStatusResponse {
  document_id: string;
  is_encrypted: boolean;
  signatures: SignatureItem[];
}

export interface SecurityActionResponse {
  success: boolean;
  document_id: string;
  message: string;
  is_encrypted?: boolean;
  signature?: SignatureItem;
}



