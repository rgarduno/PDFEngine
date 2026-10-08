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
  font_size?: number;
  font_family?: string;
  font_weight?: number;
  font_style?: string;
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
  | 'Ink'
  | 'Square'
  | 'Circle'
  | 'Line'
  | 'Polygon'
  | 'Other';

export type DrawTool = 'Ink' | 'Square' | 'Circle' | 'Line' | 'Arrow' | 'Polygon';

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
  border_width?: number;
  fill_color?: number[];
  points?: number[][];
  line_ending?: string;
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

export interface AddShapePayload {
  kind: DrawTool;
  points: number[][];
  stroke: number[];
  fill?: number[] | null;
  line_width: number;
  opacity: number;
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
  owner_password: string;
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
  certificate_pem?: string;
  private_key_pem?: string;
  chain_pem?: string;
  pkcs12_base64?: string;
  pkcs12_password?: string;
  tsa_url?: string;
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

export interface TableCellItem {
  row: number;
  col: number;
  row_span: number;
  col_span: number;
  text: string;
  is_header: boolean;
  bbox: {
    min_x: number;
    min_y: number;
    max_x: number;
    max_y: number;
    width: number;
    height: number;
  };
}

export interface DetectedTableItem {
  table_idx: number;
  page_number: number;
  row_count: number;
  col_count: number;
  bbox: {
    min_x: number;
    min_y: number;
    max_x: number;
    max_y: number;
    width: number;
    height: number;
  };
  headers: string[];
  rows: string[][];
  cells: TableCellItem[];
}

export interface PageTablesResponse {
  document_id: string;
  page_number: number;
  total_tables: number;
  tables: DetectedTableItem[];
}

export interface TableExportResponse {
  document_id: string;
  page_number: number;
  table_idx: number;
  format: string;
  content: string;
  row_count: number;
  col_count: number;
}

export interface PageOverviewItem {
  page_number: number;
  page_index: number;
  rotation: number;
  paragraph_count: number;
  preview_snippet: string;
  width: number;
  height: number;
}

export interface DocumentOverviewResponse {
  document_id: string;
  filename: string;
  total_pages: number;
  offset: number;
  limit: number;
  pages: PageOverviewItem[];
}

export interface OptimizeRequest {
  remove_unused?: boolean;
  pack_object_streams?: boolean;
  recompress_flate?: boolean;
  deduplicate_streams?: boolean;
  max_objects_per_stream?: number;
}

export interface OptimizeResponse {
  success: boolean;
  document_id: string;
  original_size: number;
  optimized_size: number;
  bytes_saved: number;
  compression_ratio_pct: number;
  objects_removed: number;
  streams_recompressed: number;
  object_streams_created: number;
  streams_deduplicated: number;
  message: string;
}

export interface OcrResponse {
  success: boolean;
  document_id: string;
  pages_seen: number;
  pages_recognized: number;
  words_inserted: number;
  message: string;
}

export interface PdfAResponse {
  success: boolean;
  document_id: string;
  part: string;
  conformance: string;
  issues: string[];
  message: string;
}

export interface CreateFormFieldPayload {
  name: string;
  field_type: FormFieldType;
  min_x: number;
  min_y: number;
  max_x: number;
  max_y: number;
  value?: string;
  default_value?: string;
  alt_name?: string;
  options?: string[];
  is_read_only?: boolean;
  is_required?: boolean;
  is_multiline?: boolean;
  max_length?: number;
  font_size?: number;
}

export interface CreateFormFieldResponse {
  status: string;
  document_id: string;
  field: FormFieldElement;
}

export interface DeleteFormFieldResponse {
  status: string;
  document_id: string;
  deleted: boolean;
  field_name: string;
}

export interface UpdateFormFieldPayload {
  min_x?: number;
  min_y?: number;
  max_x?: number;
  max_y?: number;
  alt_name?: string;
  is_read_only?: boolean;
  is_required?: boolean;
  is_multiline?: boolean;
}

export interface AuditEventItem {
  action: string;
  document_id: string;
  at: string;
}

export interface SystemCapabilities {
  tesseract_available: boolean;
  tesseract_path?: string | null;
  ocr_languages: string[];
  pdfa_supported: boolean;
  pkcs7_supported: boolean;
  concurrency_locks: boolean;
}

export interface DocumentMetadata {
  title?: string;
  author?: string;
  subject?: string;
  keywords?: string;
  creator?: string;
  producer?: string;
  creation_date?: string;
  mod_date?: string;
}

export interface DocumentMetadataResponse {
  success: boolean;
  document_id: string;
  metadata: DocumentMetadata;
}

export type DiffKind = 'unchanged' | 'added' | 'deleted' | 'modified';

export interface WordDiffItem {
  kind: DiffKind;
  text: string;
}

export interface TextDiffItem {
  kind: DiffKind;
  base_text?: string | null;
  target_text?: string | null;
  base_bbox?: [number, number, number, number] | null;
  target_bbox?: [number, number, number, number] | null;
  word_diffs: WordDiffItem[];
}

export interface ImageDiffItem {
  kind: DiffKind;
  resource_name: string;
  base_bbox?: [number, number, number, number] | null;
  target_bbox?: [number, number, number, number] | null;
  base_dimensions?: [number, number] | null;
  target_dimensions?: [number, number] | null;
}

export interface PageDimensions {
  width: number;
  height: number;
}

export interface PageDiff {
  page_number_base?: number | null;
  page_number_target?: number | null;
  kind: DiffKind;
  base_dimensions?: PageDimensions | null;
  target_dimensions?: PageDimensions | null;
  dimensions_changed: boolean;
  text_diffs: TextDiffItem[];
  image_diffs: ImageDiffItem[];
}

export interface MetadataDiffItem {
  field: string;
  base_value?: string | null;
  target_value?: string | null;
}

export interface DiffSummary {
  base_page_count: number;
  target_page_count: number;
  total_pages_with_changes: number;
  text_additions: number;
  text_deletions: number;
  text_modifications: number;
  image_additions: number;
  image_deletions: number;
  image_modifications: number;
  metadata_changes: number;
}

export interface CompareDocumentsResponse {
  base_doc_id: string;
  target_doc_id: string;
  is_identical: boolean;
  summary: DiffSummary;
  metadata_diffs: MetadataDiffItem[];
  pages: PageDiff[];
}


