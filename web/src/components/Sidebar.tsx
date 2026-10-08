'use client';

import React, { useState, useEffect } from 'react';
import { AnnotationElement, AuditEventItem, CompareDocumentsResponse, DocumentMetadata, FormFieldElement, ImageElement, Paragraph, TextDiffItem } from '@/lib/types';
import { downloadAuthorized, getImageBinaryUrl, optimizeDocument, getOptimizedExportUrl, getAuditEvents, getDocumentMetadata, updateDocumentMetadata, compareDocuments, uploadPdf } from '@/lib/api';
import { AuthorizedImage } from '@/components/AuthorizedImage';
import {
  ShieldCheck,
  Cpu,
  Layers,
  FileText,
  AlignLeft,
  ChevronRight,
  Maximize2,
  Box,
  ImageIcon,
  RefreshCw,
  RotateCw,
  RotateCcw,
  FileStack,
  Scissors,
  Trash2,
  Plus,
  Highlighter,
  Award,
  ExternalLink,
  Link2,
  Underline,
  Stamp,
  EyeOff,
  ShieldAlert,
  Lock,
  Unlock,
  Key,
  FileCheck,
  CheckCircle2,
  Table as TableIcon,
  Download,
  Copy,
  Check,
  Zap,
  FileArchive,
  Info,
  PenTool,
  History,
  Tag,
  GitCompare,
  AlertCircle,
  Upload,
} from 'lucide-react';
import {
  AddPaginationPayload,
  AddTextWatermarkPayload,
  RedactPatternPayload,
  RedactTextPayload,
  RedactRegionsPayload,
  SignatureItem,
  EncryptDocumentPayload,
  SignDocumentPayload,
  DetectedTableItem,
  OptimizeRequest,
  OptimizeResponse,
  OcrResponse,
  PdfAResponse,
  CreateFormFieldPayload,
} from '@/lib/types';

interface SidebarProps {
  paragraphs: Paragraph[];
  selectedParagraphId: number | null;
  onSelectParagraph: (id: number) => void;
  documentId: string;
  images?: ImageElement[];
  selectedImageId?: number | null;
  onSelectImage?: (id: number) => void;
  onTriggerReplaceImage?: (id: number) => void;
  forms?: FormFieldElement[];
  selectedFormFieldName?: string | null;
  onSelectFormField?: (name: string) => void;
  onUpdateFormFieldValue?: (name: string, value: string) => void;
  onFlattenForms?: () => void;
  pageNumber?: number;
  totalPages?: number;
  pageRotation?: number;
  onRotatePage?: (degrees: number) => void;
  onRotateAllPages?: (degrees: number) => void;
  onSplitDocument?: () => void;
  onTriggerMergeDocument?: () => void;
  onDeleteCurrentPage?: () => void;
  annotations?: AnnotationElement[];
  selectedAnnotationId?: number | null;
  onSelectAnnotation?: (id: number) => void;
  onAddMarkup?: (subtype: 'Highlight' | 'Underline' | 'StrikeOut') => void;
  onAddLink?: (uri: string) => void;
  onAddStamp?: (stampType: string) => void;
  onDeleteAnnotation?: (id: number) => void;
  onFlattenAnnotations?: () => void;
  onApplyPagination?: (payload: AddPaginationPayload) => void;
  onApplyTextWatermark?: (payload: AddTextWatermarkPayload) => void;
  onTriggerImageWatermark?: () => void;
  onRedactPattern?: (payload: RedactPatternPayload) => void;
  onRedactText?: (payload: RedactTextPayload) => void;
  onRedactRegions?: (payload: RedactRegionsPayload) => void;
  onSanitizeDocument?: (scrubMetadata: boolean) => void;
  isEncrypted?: boolean;
  signatures?: SignatureItem[];
  onEncryptDocument?: (payload: EncryptDocumentPayload) => void;
  onDecryptDocument?: (password: string) => void;
  onSignDocument?: (payload: SignDocumentPayload) => void;
  tables?: DetectedTableItem[];
  selectedTableIdx?: number | null;
  onSelectTable?: (idx: number | null) => void;
  onExportTable?: (tableIdx: number, format: 'csv' | 'json' | 'markdown' | 'html') => Promise<string>;
  onDownloadTable?: (tableIdx: number, format: string) => void;
  onOptimizationComplete?: (stats: OptimizeResponse) => void;
  onRecognizeScan?: (language: string) => Promise<OcrResponse>;
  onCheckPdfA?: (part: '1b' | '2b') => Promise<PdfAResponse>;
  onConvertPdfA?: (part: '1b' | '2b') => Promise<PdfAResponse>;
  onCreateFormField?: (payload: CreateFormFieldPayload) => Promise<void>;
  onDeleteFormField?: (fieldName: string) => Promise<void>;
  onNavigatePage?: (page: number) => void;
  onSetDiffHighlights?: (highlights: TextDiffItem[] | null) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  paragraphs,
  selectedParagraphId,
  onSelectParagraph,
  documentId,
  images = [],
  selectedImageId = null,
  onSelectImage,
  onTriggerReplaceImage,
  forms = [],
  selectedFormFieldName = null,
  onSelectFormField,
  onUpdateFormFieldValue,
  onFlattenForms,
  pageNumber = 1,
  totalPages = 1,
  pageRotation = 0,
  onRotatePage,
  onRotateAllPages,
  onSplitDocument,
  onTriggerMergeDocument,
  onDeleteCurrentPage,
  annotations = [],
  selectedAnnotationId = null,
  onSelectAnnotation,
  onAddMarkup,
  onAddLink,
  onAddStamp,
  onDeleteAnnotation,
  onFlattenAnnotations,
  onApplyPagination,
  onApplyTextWatermark,
  onTriggerImageWatermark,
  onRedactPattern,
  onRedactText,
  onRedactRegions,
  onSanitizeDocument,
  isEncrypted = false,
  signatures = [],
  onEncryptDocument,
  onDecryptDocument,
  onSignDocument,
  tables = [],
  selectedTableIdx = null,
  onSelectTable,
  onExportTable,
  onDownloadTable,
  onOptimizationComplete,
  onRecognizeScan,
  onCheckPdfA,
  onConvertPdfA,
  onCreateFormField,
  onDeleteFormField,
  onNavigatePage,
  onSetDiffHighlights,
}) => {
  const [activeTab, setActiveTab] = useState<'paragraphs' | 'images' | 'forms' | 'annots' | 'pages' | 'watermark' | 'redact' | 'security' | 'tables' | 'optimize' | 'ocr' | 'audit' | 'metadata' | 'diff'>('paragraphs');
  const [tableExportFormat, setTableExportFormat] = useState<'csv' | 'json' | 'markdown' | 'html'>('csv');
  const [copiedFormat, setCopiedFormat] = useState<string | null>(null);
  const [isExportingTable, setIsExportingTable] = useState<boolean>(false);
  const [linkInputUrl, setLinkInputUrl] = useState<string>('https://');

  // Diff Engine state
  const [diffTargetDocId, setDiffTargetDocId] = useState<string>('');
  const [diffReport, setDiffReport] = useState<CompareDocumentsResponse | null>(null);
  const [isComparing, setIsComparing] = useState<boolean>(false);
  const [diffError, setDiffError] = useState<string | null>(null);
  const [diffIgnoreCase, setDiffIgnoreCase] = useState<boolean>(false);
  const [diffCompareImages, setDiffCompareImages] = useState<boolean>(true);
  const [diffCompareMetadata, setDiffCompareMetadata] = useState<boolean>(true);

  const handleRunComparison = async () => {
    if (!documentId || !diffTargetDocId.trim()) return;
    try {
      setIsComparing(true);
      setDiffError(null);
      const res = await compareDocuments(documentId, diffTargetDocId.trim(), {
        ignore_case: diffIgnoreCase,
        compare_images: diffCompareImages,
        compare_metadata: diffCompareMetadata,
      });
      setDiffReport(res);
      const pageDiff = res.pages.find((p) => (p.page_number_target || p.page_number_base) === pageNumber);
      if (pageDiff && pageDiff.text_diffs.length > 0) {
        onSetDiffHighlights?.(pageDiff.text_diffs);
      }
    } catch (err: unknown) {
      console.error('Failed to run comparison:', err);
      setDiffError((err as Error).message || 'Error comparando documentos.');
    } finally {
      setIsComparing(false);
    }
  };

  const handleUploadDiffTarget = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      setIsComparing(true);
      setDiffError(null);
      const sessionRes = await uploadPdf(file);
      setDiffTargetDocId(sessionRes.document_id);
      const res = await compareDocuments(documentId, sessionRes.document_id, {
        ignore_case: diffIgnoreCase,
        compare_images: diffCompareImages,
        compare_metadata: diffCompareMetadata,
      });
      setDiffReport(res);
      const pageDiff = res.pages.find((p) => (p.page_number_target || p.page_number_base) === pageNumber);
      if (pageDiff && pageDiff.text_diffs.length > 0) {
        onSetDiffHighlights?.(pageDiff.text_diffs);
      }
    } catch (err: unknown) {
      console.error('Failed to upload and compare target:', err);
      setDiffError((err as Error).message || 'Error subiendo y comparando documento target.');
    } finally {
      setIsComparing(false);
    }
  };

  // Metadata state (/Info & XMP)
  const [metadata, setMetadata] = useState<DocumentMetadata>({});
  const [loadingMetadata, setLoadingMetadata] = useState<boolean>(false);
  const [savingMetadata, setSavingMetadata] = useState<boolean>(false);
  const [metadataFeedback, setMetadataFeedback] = useState<string | null>(null);
  const [metadataError, setMetadataError] = useState<string | null>(null);

  const loadMetadata = async () => {
    if (!documentId) return;
    try {
      setLoadingMetadata(true);
      setMetadataError(null);
      const res = await getDocumentMetadata(documentId);
      setMetadata(res.metadata || {});
    } catch (err: unknown) {
      console.error('Failed to load document metadata:', err);
      setMetadataError((err as Error).message || 'No se pudieron cargar los metadatos.');
    } finally {
      setLoadingMetadata(false);
    }
  };

  const handleSaveMetadata = async () => {
    if (!documentId) return;
    try {
      setSavingMetadata(true);
      setMetadataFeedback(null);
      setMetadataError(null);
      const res = await updateDocumentMetadata(documentId, metadata);
      setMetadata(res.metadata || {});
      setMetadataFeedback('Metadatos sincronizados con éxito (/Info y XMP).');
      setTimeout(() => setMetadataFeedback(null), 4000);
    } catch (err: unknown) {
      console.error('Failed to update metadata:', err);
      setMetadataError((err as Error).message || 'Error guardando metadatos.');
    } finally {
      setSavingMetadata(false);
    }
  };

  // Audit log state
  const [auditEvents, setAuditEvents] = useState<AuditEventItem[]>([]);
  const [isLoadingAudit, setIsLoadingAudit] = useState<boolean>(false);
  const [auditError, setAuditError] = useState<string | null>(null);

  const loadAuditLogs = async () => {
    try {
      setIsLoadingAudit(true);
      setAuditError(null);
      const events = await getAuditEvents();
      setAuditEvents(events);
    } catch (err) {
      console.error('Failed to load audit events:', err);
      setAuditError('No se pudo cargar la pista de auditoría.');
    } finally {
      setIsLoadingAudit(false);
    }
  };

  useEffect(() => {
    if (activeTab === 'audit') {
      void loadAuditLogs();
    } else if (activeTab === 'metadata') {
      void loadMetadata();
    }
  }, [activeTab, documentId]);

  const [pagFormat, setPagFormat] = useState<string>('Página {page} de {total}');
  const [pagPosition, setPagPosition] = useState<'top_left' | 'top_center' | 'top_right' | 'bottom_left' | 'bottom_center' | 'bottom_right'>('bottom_center');
  const [pagFontSize, setPagFontSize] = useState<number>(9);
  const [pagMargin, setPagMargin] = useState<number>(36);
  const [pagSkipFirst, setPagSkipFirst] = useState<boolean>(false);

  // PDF Optimization & Compression state
  const [optRemoveUnused, setOptRemoveUnused] = useState<boolean>(true);
  const [optPackObjectStreams, setOptPackObjectStreams] = useState<boolean>(true);
  const [optRecompressFlate, setOptRecompressFlate] = useState<boolean>(true);
  const [optDeduplicateStreams, setOptDeduplicateStreams] = useState<boolean>(true);
  const [optMaxObjectsPerStream, setOptMaxObjectsPerStream] = useState<number>(100);
  const [isOptimizing, setIsOptimizing] = useState<boolean>(false);
  const [optimizeStats, setOptimizeStats] = useState<OptimizeResponse | null>(null);
  const [optimizeError, setOptimizeError] = useState<string | null>(null);
  const [copiedOptReport, setCopiedOptReport] = useState<boolean>(false);
  const [ocrLanguage, setOcrLanguage] = useState<string>('');
  const [ocrBusy, setOcrBusy] = useState<boolean>(false);
  const [ocrResult, setOcrResult] = useState<OcrResponse | null>(null);
  const [ocrError, setOcrError] = useState<string | null>(null);
  const [pdfaPart, setPdfaPart] = useState<'1b' | '2b'>('1b');
  const [pdfaBusy, setPdfaBusy] = useState<'check' | 'convert' | null>(null);
  const [pdfaResult, setPdfaResult] = useState<PdfAResponse | null>(null);
  const [pdfaError, setPdfaError] = useState<string | null>(null);

  const [wmText, setWmText] = useState<string>('CONFIDENCIAL');
  const [wmFontSize, setWmFontSize] = useState<number>(52);
  const [wmOpacity, setWmOpacity] = useState<number>(0.22);
  const [wmRotation, setWmRotation] = useState<number>(45);
  const [wmPlacement, setWmPlacement] = useState<'background' | 'foreground'>('background');
  const [wmTargetAll, setWmTargetAll] = useState<boolean>(true);

  // Redaction & PII Sanitizer State
  const [redactSubMode, setRedactSubMode] = useState<'pattern' | 'text' | 'region'>('pattern');
  const [redactPatternType, setRedactPatternType] = useState<'email' | 'phone' | 'rfc' | 'curp' | 'credit_card' | 'ssn'>('email');
  const [redactQueryText, setRedactQueryText] = useState<string>('');
  const [redactOverlayLabel, setRedactOverlayLabel] = useState<string>('[REDACTADO]');
  const [redactTargetAll, setRedactTargetAll] = useState<boolean>(true);
  const [redactPruneAnnotations, setRedactPruneAnnotations] = useState<boolean>(true);
  const [redactScrubMetadata, setRedactScrubMetadata] = useState<boolean>(true);
  const [redactBoxMinX, setRedactBoxMinX] = useState<number>(72);
  const [redactBoxMinY, setRedactBoxMinY] = useState<number>(700);
  const [redactBoxMaxX, setRedactBoxMaxX] = useState<number>(250);
  const [redactBoxMaxY, setRedactBoxMaxY] = useState<number>(720);

  // Security & Signature State
  const [secUserPass, setSecUserPass] = useState<string>('');
  const [secOwnerPass, setSecOwnerPass] = useState<string>('');
  const [secDecryptPass, setSecDecryptPass] = useState<string>('');
  const [secPermPrintHigh, setSecPermPrintHigh] = useState<boolean>(true);
  const [secPermModifyContents, setSecPermModifyContents] = useState<boolean>(false);
  const [secPermCopyExtract, setSecPermCopyExtract] = useState<boolean>(false);
  const [secPermModifyAnnots, setSecPermModifyAnnots] = useState<boolean>(true);
  const [secPermFillForms, setSecPermFillForms] = useState<boolean>(true);
  const [secPermAccessibility, setSecPermAccessibility] = useState<boolean>(true);
  const [secPermAssemble, setSecPermAssemble] = useState<boolean>(false);

  const [sigName, setSigName] = useState<string>('Lic. Roberto Garduño');
  const [sigReason, setSigReason] = useState<string>('Aprobación y Certificación Legal');
  const [sigLocation, setSigLocation] = useState<string>('Ciudad de México, MX');
  const [sigPage, setSigPage] = useState<number>(pageNumber || 1);
  const [sigFormat, setSigFormat] = useState<'attestation' | 'pkcs12' | 'pem'>('attestation');
  const [sigPkcs12Base64, setSigPkcs12Base64] = useState<string>('');
  const [sigPkcs12Filename, setSigPkcs12Filename] = useState<string>('');
  const [sigPkcs12Password, setSigPkcs12Password] = useState<string>('');
  const [sigCertPem, setSigCertPem] = useState<string>('');
  const [sigKeyPem, setSigKeyPem] = useState<string>('');
  const [sigTsaUrl, setSigTsaUrl] = useState<string>('');
  const [showAdvancedCrypto, setShowAdvancedCrypto] = useState<boolean>(false);

  // AcroForm Builder & Designer State
  const [showFormBuilder, setShowFormBuilder] = useState<boolean>(false);
  const [newFieldName, setNewFieldName] = useState<string>('');
  const [newFieldAltName, setNewFieldAltName] = useState<string>('');
  const [newFieldType, setNewFieldType] = useState<'Text' | 'Checkbox' | 'Choice' | 'Signature'>('Text');
  const [newFieldMinX, setNewFieldMinX] = useState<number>(72);
  const [newFieldMinY, setNewFieldMinY] = useState<number>(650);
  const [newFieldWidth, setNewFieldWidth] = useState<number>(180);
  const [newFieldHeight, setNewFieldHeight] = useState<number>(24);
  const [newFieldValue, setNewFieldValue] = useState<string>('');
  const [newFieldOptions, setNewFieldOptions] = useState<string>('Opción 1, Opción 2, Opción 3');
  const [newFieldRequired, setNewFieldRequired] = useState<boolean>(false);
  const [newFieldReadOnly, setNewFieldReadOnly] = useState<boolean>(false);
  const [newFieldMultiline, setNewFieldMultiline] = useState<boolean>(false);
  const [isSubmittingForm, setIsSubmittingForm] = useState<boolean>(false);

  const applyFieldPreset = (preset: 'text' | 'textarea' | 'checkbox' | 'choice' | 'signature') => {
    switch (preset) {
      case 'text':
        setNewFieldType('Text');
        setNewFieldWidth(180);
        setNewFieldHeight(24);
        setNewFieldMultiline(false);
        if (!newFieldName) setNewFieldName('campo_texto');
        break;
      case 'textarea':
        setNewFieldType('Text');
        setNewFieldWidth(220);
        setNewFieldHeight(60);
        setNewFieldMultiline(true);
        if (!newFieldName) setNewFieldName('comentarios');
        break;
      case 'checkbox':
        setNewFieldType('Checkbox');
        setNewFieldWidth(18);
        setNewFieldHeight(18);
        setNewFieldMultiline(false);
        if (!newFieldName) setNewFieldName('acepto_terminos');
        break;
      case 'choice':
        setNewFieldType('Choice');
        setNewFieldWidth(180);
        setNewFieldHeight(24);
        setNewFieldMultiline(false);
        if (!newFieldName) setNewFieldName('seleccion');
        break;
      case 'signature':
        setNewFieldType('Signature');
        setNewFieldWidth(200);
        setNewFieldHeight(50);
        setNewFieldMultiline(false);
        if (!newFieldName) setNewFieldName('firma_digital');
        break;
    }
  };

  const handleCreateNewField = async () => {
    if (!newFieldName.trim()) {
      alert('Por favor especifica un nombre único para el campo.');
      return;
    }
    if (!onCreateFormField) return;

    setIsSubmittingForm(true);
    try {
      const optionsArray = newFieldType === 'Choice'
        ? newFieldOptions.split(',').map((s) => s.trim()).filter(Boolean)
        : undefined;

      const payload: CreateFormFieldPayload = {
        name: newFieldName.trim(),
        alt_name: newFieldAltName.trim() || undefined,
        field_type: newFieldType,
        value: newFieldValue || undefined,
        min_x: newFieldMinX,
        min_y: newFieldMinY,
        max_x: newFieldMinX + newFieldWidth,
        max_y: newFieldMinY + newFieldHeight,
        is_required: newFieldRequired,
        is_read_only: newFieldReadOnly,
        is_multiline: newFieldType === 'Text' ? newFieldMultiline : undefined,
        options: optionsArray,
      };

      await onCreateFormField(payload);
      setShowFormBuilder(false);
      setNewFieldName('');
      setNewFieldAltName('');
      setNewFieldValue('');
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Error al crear campo';
      alert(`Error al crear campo: ${msg}`);
    } finally {
      setIsSubmittingForm(false);
    }
  };

  const handleRecognizeScan = async () => {
    const language = ocrLanguage.trim();
    if (language.length > 16) {
      setOcrError('El idioma admite como máximo 16 caracteres.');
      return;
    }
    if (!onRecognizeScan) {
      setOcrError('El estudio no tiene la acción de reconocimiento.');
      return;
    }
    setOcrBusy(true);
    setOcrError(null);
    try {
      const response = await onRecognizeScan(language);
      setOcrResult(response);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Error al reconocer el escaneo';
      setOcrError(msg);
    } finally {
      setOcrBusy(false);
    }
  };

  const handleCheckPdfA = async () => {
    if (!onCheckPdfA) {
      setPdfaError('El estudio no tiene la revisión de archivo.');
      return;
    }
    setPdfaBusy('check');
    setPdfaError(null);
    try {
      const response = await onCheckPdfA(pdfaPart);
      setPdfaResult(response);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Error al revisar el archivo';
      setPdfaError(msg);
    } finally {
      setPdfaBusy(null);
    }
  };

  const handleConvertPdfA = async () => {
    if (!onConvertPdfA) {
      setPdfaError('El estudio no tiene la conversión de archivo.');
      return;
    }
    setPdfaBusy('convert');
    setPdfaError(null);
    try {
      const response = await onConvertPdfA(pdfaPart);
      setPdfaResult(response);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Error al convertir el archivo';
      setPdfaError(msg);
    } finally {
      setPdfaBusy(null);
    }
  };

  const handleRunOptimization = async () => {
    setIsOptimizing(true);
    setOptimizeError(null);
    try {
      const payload: OptimizeRequest = {
        remove_unused: optRemoveUnused,
        pack_object_streams: optPackObjectStreams,
        recompress_flate: optRecompressFlate,
        deduplicate_streams: optDeduplicateStreams,
        max_objects_per_stream: optMaxObjectsPerStream,
      };
      const response = await optimizeDocument(documentId, payload);
      setOptimizeStats(response);
      onOptimizationComplete?.(response);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Error al optimizar documento';
      setOptimizeError(msg);
    } finally {
      setIsOptimizing(false);
    }
  };

  const formatOptBytes = (bytes: number): string => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
  };

  const handleCopyOptReport = () => {
    if (!optimizeStats) return;
    const report = [
      `=== REPORTE DE OPTIMIZACIÓN PDF (ISO 32000-1) ===`,
      `Documento ID: ${documentId}`,
      `Tamaño Original: ${formatOptBytes(optimizeStats.original_size)}`,
      `Tamaño Optimizado: ${formatOptBytes(optimizeStats.optimized_size)}`,
      `Ahorro Total: ${formatOptBytes(optimizeStats.bytes_saved)} (${optimizeStats.compression_ratio_pct.toFixed(1)}%)`,
      `Objetos Huérfanos Purgados: ${optimizeStats.objects_removed}`,
      `Flujos Recomprimidos: ${optimizeStats.streams_recompressed}`,
      `Contenedores /ObjStm Creados: ${optimizeStats.object_streams_created}`,
      `Flujos Deduplicados: ${optimizeStats.streams_deduplicated}`,
    ].join('\n');
    navigator.clipboard.writeText(report);
    setCopiedOptReport(true);
    setTimeout(() => setCopiedOptReport(false), 2000);
  };

  return (
    <aside className="w-80 border-l border-neutral-200 dark:border-neutral-800 bg-white/95 dark:bg-neutral-900/95 flex flex-col h-[calc(100vh-4rem)] select-none">
      {/* Header */}
      <div className="p-4 border-b border-neutral-200 dark:border-neutral-800">
        <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-neutral-500">
          <Layers size={14} />
          <span>SceneGraph Inspector</span>
        </div>
        <div className="mt-1 flex items-center justify-between text-xs text-neutral-400 font-mono">
          <span>ID: {documentId.slice(0, 12)}...</span>
          <span className="text-emerald-500 font-medium">AST Synced</span>
        </div>

        {/* Tab Switcher */}
        <div className="mt-3 grid grid-cols-7 gap-1 p-1 bg-neutral-100 dark:bg-neutral-800 rounded-lg">
          <button
            onClick={() => setActiveTab('paragraphs')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'paragraphs'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
            title="Blocks"
          >
            <AlignLeft size={10} />
            <span>Blocks</span>
          </button>
          <button
            onClick={() => setActiveTab('images')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'images'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
            title="Images"
          >
            <ImageIcon size={10} />
            <span>Imgs</span>
          </button>
          <button
            onClick={() => setActiveTab('forms')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'forms'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
            title="Forms"
          >
            <FileText size={10} />
            <span>Forms</span>
          </button>
          <button
            onClick={() => setActiveTab('annots')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'annots'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
            title="Annotations"
          >
            <Highlighter size={10} />
            <span>Marks</span>
          </button>
          <button
            onClick={() => setActiveTab('pages')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'pages'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
            title="Pages"
          >
            <FileStack size={10} />
            <span>Pages</span>
          </button>
          <button
            onClick={() => setActiveTab('watermark')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'watermark'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
            title="Watermark & Pagination"
          >
            <Stamp size={10} />
            <span>Folio</span>
          </button>
          <button
            onClick={() => setActiveTab('metadata')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'metadata'
                ? 'bg-teal-600 text-white font-semibold shadow-xs'
                : 'text-teal-600 hover:text-teal-700 dark:hover:text-teal-400'
            }`}
            title="Editor y Sincronizador de Metadatos Documentales (/Info & XMP)"
          >
            <Tag size={10} />
            <span>Meta</span>
          </button>
          <button
            onClick={() => setActiveTab('redact')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'redact'
                ? 'bg-rose-600 text-white font-semibold shadow-xs'
                : 'text-rose-600 hover:text-rose-700 dark:hover:text-rose-400'
            }`}
            title="Censura Quirúrgica e Irreversible"
          >
            <EyeOff size={10} />
            <span>Censura</span>
          </button>
          <button
            onClick={() => setActiveTab('security')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'security'
                ? 'bg-amber-600 text-white font-semibold shadow-xs'
                : 'text-amber-600 hover:text-amber-700 dark:hover:text-amber-400'
            }`}
            title="Cifrado, Permisos y Firmas Digitales"
          >
            <Lock size={10} />
            <span>Seguro</span>
          </button>
          <button
            onClick={() => setActiveTab('tables')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'tables'
                ? 'bg-emerald-600 text-white font-semibold shadow-xs'
                : 'text-emerald-600 hover:text-emerald-700 dark:hover:text-emerald-400'
            }`}
            title="Reconocimiento y Extracción de Tablas (ISO 32000 §14.8.4)"
          >
            <TableIcon size={10} />
            <span>Tablas</span>
          </button>
          <button
            onClick={() => setActiveTab('optimize')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'optimize'
                ? 'bg-amber-500 text-white font-semibold shadow-xs'
                : 'text-amber-600 hover:text-amber-700 dark:hover:text-amber-400'
            }`}
            title="Optimización y Compresión ISO 32000-1 §7.5.7 (/ObjStm)"
          >
            <Zap size={10} />
            <span>Optim</span>
          </button>
          <button
            onClick={() => setActiveTab('ocr')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'ocr'
                ? 'bg-sky-600 text-white font-semibold shadow-xs'
                : 'text-sky-700 hover:text-sky-800 dark:hover:text-sky-300'
            }`}
            title="Texto buscable y PDF/A"
          >
            <FileCheck size={10} />
            <span>OCR</span>
          </button>
          <button
            onClick={() => setActiveTab('audit')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'audit'
                ? 'bg-indigo-600 text-white font-semibold shadow-xs'
                : 'text-indigo-600 hover:text-indigo-700 dark:hover:text-indigo-400'
            }`}
            title="Pista de Auditoría (/api/audit)"
          >
            <History size={10} />
            <span>Audit</span>
          </button>
          <button
            onClick={() => setActiveTab('diff')}
            className={`flex items-center justify-center gap-0.5 py-1 text-[8px] font-medium rounded-md transition-all ${
              activeTab === 'diff'
                ? 'bg-purple-600 text-white font-semibold shadow-xs'
                : 'text-purple-600 hover:text-purple-700 dark:hover:text-purple-400'
            }`}
            title="Comparador Semántico y Auditoría de Revisiones (Diff Engine)"
          >
            <GitCompare size={10} />
            <span>Diff</span>
          </button>
        </div>
      </div>

      {/* Content List */}
      <div className="flex-1 overflow-y-auto p-3 space-y-2">
        {activeTab === 'paragraphs' ? (
          <>
            <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-1">
              Detected Paragraph Blocks ({paragraphs.length})
            </div>

            {paragraphs.map((p) => {
              const isSelected = selectedParagraphId === p.id;
              return (
                <div
                  key={p.id}
                  onClick={() => onSelectParagraph(p.id)}
                  className={`p-3 rounded-lg border text-left cursor-pointer transition-all ${
                    isSelected
                      ? 'border-blue-500 bg-blue-50/50 dark:bg-blue-900/20 shadow-xs'
                      : 'border-neutral-200 dark:border-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 bg-neutral-50/50 dark:bg-neutral-800/30'
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <span className="font-mono text-xs font-semibold text-blue-600 dark:text-blue-400 flex items-center gap-1">
                      <Box size={12} />
                      Block #{p.id}
                    </span>
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-700 text-neutral-600 dark:text-neutral-300 uppercase">
                      {p.alignment}
                    </span>
                  </div>

                  <p className="mt-1.5 text-xs text-neutral-700 dark:text-neutral-300 line-clamp-2 leading-relaxed">
                    {p.text}
                  </p>

                  <div className="mt-2 pt-2 border-t border-neutral-200/60 dark:border-neutral-700/60 flex items-center justify-between text-[11px] text-neutral-400 font-mono">
                    <span>{p.line_count} line{p.line_count > 1 ? 's' : ''}</span>
                    <span>
                      {Math.round(p.bbox.width)}x{Math.round(p.bbox.height)} pt
                    </span>
                  </div>
                </div>
              );
            })}
          </>
        ) : activeTab === 'images' ? (
          <>
            <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-1">
              XObject Images ({images.length})
            </div>

            {images.length === 0 ? (
              <div className="py-8 text-center text-xs text-neutral-400">
                No XObject images detected on this page.
              </div>
            ) : (
              images.map((img) => {
                const isSelected = selectedImageId === img.id;
                const imageUrl = getImageBinaryUrl(documentId, img.id);

                return (
                  <div
                    key={img.id}
                    onClick={() => onSelectImage?.(img.id)}
                    className={`p-3 rounded-lg border text-left cursor-pointer transition-all ${
                      isSelected
                        ? 'border-emerald-500 bg-emerald-50/50 dark:bg-emerald-900/20 shadow-xs'
                        : 'border-neutral-200 dark:border-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 bg-neutral-50/50 dark:bg-neutral-800/30'
                    }`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-mono text-xs font-semibold text-emerald-600 dark:text-emerald-400 flex items-center gap-1">
                        <ImageIcon size={12} />
                        Img #{img.id}
                      </span>
                      <span className="text-[10px] px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-700 text-neutral-600 dark:text-neutral-300 font-mono">
                        {img.filter || 'Raw'}
                      </span>
                    </div>

                    {/* Image Thumbnail */}
                    <div className="mt-2 w-full h-24 rounded overflow-hidden bg-neutral-100 dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 flex items-center justify-center">
                      <AuthorizedImage
                        url={imageUrl}
                        alt={`XObject #${img.id}`}
                        className="w-full h-full object-contain"
                      />
                    </div>

                    {/* Image Metadata */}
                    <div className="mt-2 space-y-1 text-[11px] font-mono text-neutral-500 dark:text-neutral-400">
                      <div className="flex justify-between">
                        <span>Resolution:</span>
                        <span className="text-neutral-700 dark:text-neutral-200 font-medium">
                          {img.width_px} × {img.height_px} px
                        </span>
                      </div>
                      <div className="flex justify-between">
                        <span>Color Space:</span>
                        <span className="text-neutral-700 dark:text-neutral-200 font-medium">
                          {img.color_space}
                        </span>
                      </div>
                      <div className="flex justify-between">
                        <span>BBox Size:</span>
                        <span className="text-neutral-700 dark:text-neutral-200 font-medium">
                          {Math.round(img.bbox.width)} × {Math.round(img.bbox.height)} pt
                        </span>
                      </div>
                    </div>

                    {/* Replace Action Button */}
                    {onTriggerReplaceImage && (
                      <div className="mt-2 pt-2 border-t border-neutral-200/60 dark:border-neutral-700/60">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            onTriggerReplaceImage(img.id);
                          }}
                          className="w-full flex items-center justify-center gap-1.5 py-1.5 text-xs font-medium bg-emerald-600 hover:bg-emerald-700 text-white rounded transition-colors"
                        >
                          <RefreshCw size={12} />
                          <span>Replace Image</span>
                        </button>
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </>
        ) : activeTab === 'forms' ? (
          <>
            <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-2 flex items-center justify-between">
              <span>Campos de Formulario ({forms.length})</span>
              {onCreateFormField && (
                <button
                  type="button"
                  onClick={() => setShowFormBuilder((prev) => !prev)}
                  className="flex items-center gap-1 text-[11px] font-medium text-purple-600 hover:text-purple-700 dark:text-purple-400 dark:hover:text-purple-300 transition-colors cursor-pointer"
                >
                  <Plus size={12} />
                  <span>{showFormBuilder ? 'Cerrar' : '+ Nuevo Campo'}</span>
                </button>
              )}
            </div>

            {/* Form Builder Drawer / Card */}
            {showFormBuilder && (
              <div className="p-3 mb-3 rounded-lg border border-purple-200 dark:border-purple-800 bg-purple-50/60 dark:bg-purple-950/30 space-y-3">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-1.5 text-xs font-semibold text-purple-900 dark:text-purple-200">
                    <Box size={14} className="text-purple-600 dark:text-purple-400" />
                    <span>Diseñador de Campos</span>
                  </div>
                  <span className="text-[10px] px-1.5 py-0.5 rounded bg-purple-200 dark:bg-purple-800 text-purple-800 dark:text-purple-200 font-mono">
                    Pág. {pageNumber}
                  </span>
                </div>

                {/* Presets */}
                <div className="space-y-1">
                  <span className="text-[10px] text-neutral-500 uppercase tracking-wider font-semibold">Presets Rápidos:</span>
                  <div className="grid grid-cols-3 gap-1">
                    <button
                      type="button"
                      onClick={() => applyFieldPreset('text')}
                      className={`text-[10px] py-1 px-1.5 rounded border transition-colors ${
                        newFieldType === 'Text' && !newFieldMultiline
                          ? 'bg-purple-600 text-white border-purple-600 font-medium'
                          : 'bg-white dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-700 hover:border-purple-400'
                      }`}
                    >
                      Texto
                    </button>
                    <button
                      type="button"
                      onClick={() => applyFieldPreset('textarea')}
                      className={`text-[10px] py-1 px-1.5 rounded border transition-colors ${
                        newFieldType === 'Text' && newFieldMultiline
                          ? 'bg-purple-600 text-white border-purple-600 font-medium'
                          : 'bg-white dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-700 hover:border-purple-400'
                      }`}
                    >
                      Área Texto
                    </button>
                    <button
                      type="button"
                      onClick={() => applyFieldPreset('checkbox')}
                      className={`text-[10px] py-1 px-1.5 rounded border transition-colors ${
                        newFieldType === 'Checkbox'
                          ? 'bg-purple-600 text-white border-purple-600 font-medium'
                          : 'bg-white dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-700 hover:border-purple-400'
                      }`}
                    >
                      Checkbox
                    </button>
                    <button
                      type="button"
                      onClick={() => applyFieldPreset('choice')}
                      className={`text-[10px] py-1 px-1.5 rounded border transition-colors ${
                        newFieldType === 'Choice'
                          ? 'bg-purple-600 text-white border-purple-600 font-medium'
                          : 'bg-white dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-700 hover:border-purple-400'
                      }`}
                    >
                      Lista / Combo
                    </button>
                    <button
                      type="button"
                      onClick={() => applyFieldPreset('signature')}
                      className={`text-[10px] py-1 px-1.5 rounded border transition-colors col-span-2 ${
                        newFieldType === 'Signature'
                          ? 'bg-purple-600 text-white border-purple-600 font-medium'
                          : 'bg-white dark:bg-neutral-800 text-neutral-700 dark:text-neutral-300 border-neutral-200 dark:border-neutral-700 hover:border-purple-400'
                      }`}
                    >
                      Firma Digital (/Sig)
                    </button>
                  </div>
                </div>

                {/* Field Name & Alt Name */}
                <div className="space-y-1.5">
                  <div>
                    <label className="text-[10px] text-neutral-600 dark:text-neutral-400 block font-medium">
                      Nombre del Campo (/T) *
                    </label>
                    <input
                      type="text"
                      value={newFieldName}
                      onChange={(e) => setNewFieldName(e.target.value)}
                      placeholder="ej. nombre_cliente"
                      className="w-full text-xs p-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 outline-none focus:border-purple-500 font-mono"
                    />
                  </div>
                  <div>
                    <label className="text-[10px] text-neutral-600 dark:text-neutral-400 block font-medium">
                      Etiqueta / Tooltip (/TU)
                    </label>
                    <input
                      type="text"
                      value={newFieldAltName}
                      onChange={(e) => setNewFieldAltName(e.target.value)}
                      placeholder="ej. Ingrese su nombre completo"
                      className="w-full text-xs p-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 outline-none focus:border-purple-500"
                    />
                  </div>
                </div>

                {/* Coordinates & Dimensions */}
                <div className="space-y-1">
                  <label className="text-[10px] text-neutral-600 dark:text-neutral-400 block font-medium">
                    Posición y Tamaño (pt)
                  </label>
                  <div className="grid grid-cols-4 gap-1">
                    <div>
                      <span className="text-[9px] text-neutral-400 block">X</span>
                      <input
                        type="number"
                        value={newFieldMinX}
                        onChange={(e) => setNewFieldMinX(Number(e.target.value))}
                        className="w-full text-xs p-1 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
                      />
                    </div>
                    <div>
                      <span className="text-[9px] text-neutral-400 block">Y</span>
                      <input
                        type="number"
                        value={newFieldMinY}
                        onChange={(e) => setNewFieldMinY(Number(e.target.value))}
                        className="w-full text-xs p-1 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
                      />
                    </div>
                    <div>
                      <span className="text-[9px] text-neutral-400 block">Ancho</span>
                      <input
                        type="number"
                        value={newFieldWidth}
                        onChange={(e) => setNewFieldWidth(Number(e.target.value))}
                        className="w-full text-xs p-1 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
                      />
                    </div>
                    <div>
                      <span className="text-[9px] text-neutral-400 block">Alto</span>
                      <input
                        type="number"
                        value={newFieldHeight}
                        onChange={(e) => setNewFieldHeight(Number(e.target.value))}
                        className="w-full text-xs p-1 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 font-mono"
                      />
                    </div>
                  </div>
                </div>

                {/* Specifics depending on field type */}
                {newFieldType === 'Choice' && (
                  <div>
                    <label className="text-[10px] text-neutral-600 dark:text-neutral-400 block font-medium">
                      Opciones (separadas por coma)
                    </label>
                    <input
                      type="text"
                      value={newFieldOptions}
                      onChange={(e) => setNewFieldOptions(e.target.value)}
                      placeholder="Opción 1, Opción 2, Opción 3"
                      className="w-full text-xs p-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 outline-none focus:border-purple-500"
                    />
                  </div>
                )}

                {newFieldType !== 'Signature' && (
                  <div>
                    <label className="text-[10px] text-neutral-600 dark:text-neutral-400 block font-medium">
                      Valor Inicial
                    </label>
                    <input
                      type="text"
                      value={newFieldValue}
                      onChange={(e) => setNewFieldValue(e.target.value)}
                      placeholder={newFieldType === 'Checkbox' ? 'Yes / Off' : 'Texto por defecto'}
                      className="w-full text-xs p-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 outline-none focus:border-purple-500"
                    />
                  </div>
                )}

                {/* Flags: Required, ReadOnly, Multiline */}
                <div className="flex flex-wrap gap-3 pt-1">
                  <label className="flex items-center gap-1.5 text-[11px] text-neutral-700 dark:text-neutral-300 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={newFieldRequired}
                      onChange={(e) => setNewFieldRequired(e.target.checked)}
                      className="w-3.5 h-3.5 text-purple-600 rounded focus:ring-0 cursor-pointer"
                    />
                    <span>Requerido</span>
                  </label>
                  <label className="flex items-center gap-1.5 text-[11px] text-neutral-700 dark:text-neutral-300 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={newFieldReadOnly}
                      onChange={(e) => setNewFieldReadOnly(e.target.checked)}
                      className="w-3.5 h-3.5 text-purple-600 rounded focus:ring-0 cursor-pointer"
                    />
                    <span>Solo Lectura</span>
                  </label>
                  {newFieldType === 'Text' && (
                    <label className="flex items-center gap-1.5 text-[11px] text-neutral-700 dark:text-neutral-300 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={newFieldMultiline}
                        onChange={(e) => setNewFieldMultiline(e.target.checked)}
                        className="w-3.5 h-3.5 text-purple-600 rounded focus:ring-0 cursor-pointer"
                      />
                      <span>Multilínea</span>
                    </label>
                  )}
                </div>

                {/* Create & Cancel Buttons */}
                <div className="flex gap-2 pt-1">
                  <button
                    type="button"
                    disabled={isSubmittingForm || !newFieldName.trim()}
                    onClick={handleCreateNewField}
                    className="flex-1 py-1.5 px-3 bg-purple-600 hover:bg-purple-700 disabled:opacity-50 text-white rounded text-xs font-semibold shadow-xs transition-colors flex items-center justify-center gap-1.5"
                  >
                    <Plus size={13} />
                    <span>{isSubmittingForm ? 'Insertando...' : 'Insertar en PDF'}</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => setShowFormBuilder(false)}
                    className="py-1.5 px-2.5 bg-neutral-200 dark:bg-neutral-800 hover:bg-neutral-300 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-300 rounded text-xs font-medium transition-colors"
                  >
                    Cancelar
                  </button>
                </div>
              </div>
            )}

            {forms.length === 0 ? (
              <div className="py-8 text-center text-xs text-neutral-400">
                No hay campos de formulario interactivos en el documento.
              </div>
            ) : (
              <div className="space-y-2">
                {forms.map((f) => {
                  const isSelected = selectedFormFieldName === f.name;
                  return (
                    <div
                      key={f.id}
                      onClick={() => onSelectFormField?.(f.name)}
                      className={`p-3 rounded-lg border text-left cursor-pointer transition-all ${
                        isSelected
                          ? 'border-purple-500 bg-purple-50/50 dark:bg-purple-900/20 shadow-xs'
                          : 'border-neutral-200 dark:border-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 bg-neutral-50/50 dark:bg-neutral-800/30'
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <span className="font-mono text-xs font-semibold text-purple-600 dark:text-purple-400 flex items-center gap-1 truncate max-w-[130px]">
                          <FileText size={12} />
                          {f.name}
                        </span>
                        <div className="flex items-center gap-1">
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-purple-100 dark:bg-purple-950/60 text-purple-700 dark:text-purple-300 font-mono">
                            {f.field_type}
                          </span>
                          {onDeleteFormField && (
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation();
                                onDeleteFormField(f.name);
                              }}
                              title="Eliminar campo"
                              className="p-1 text-neutral-400 hover:text-red-500 rounded transition-colors"
                            >
                              <Trash2 size={12} />
                            </button>
                          )}
                        </div>
                      </div>

                      {f.alt_name && (
                        <div className="text-[11px] text-neutral-500 mt-1 italic">
                          {f.alt_name}
                        </div>
                      )}

                      {/* Field Value Input directly in Sidebar */}
                      <div className="mt-2" onClick={(e) => e.stopPropagation()}>
                        {f.field_type === 'Signature' ? (
                          <div className="p-2 bg-purple-100/50 dark:bg-purple-950/50 border border-purple-300 dark:border-purple-800 rounded text-xs text-purple-800 dark:text-purple-200 flex items-center gap-2">
                            <PenTool size={13} className="text-purple-600 dark:text-purple-400 shrink-0" />
                            <span className="truncate">Campo de Firma Digital {f.value ? `(${f.value})` : '(Pendiente)'}</span>
                          </div>
                        ) : f.field_type === 'Checkbox' ? (
                          <label className="flex items-center gap-2 text-xs text-neutral-700 dark:text-neutral-300 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={
                                f.value.toLowerCase() === 'yes' ||
                                f.value === '1' ||
                                f.value.toLowerCase() === 'true'
                              }
                              onChange={(e) =>
                                onUpdateFormFieldValue?.(f.name, e.target.checked ? 'Yes' : 'Off')
                              }
                              className="w-4 h-4 text-purple-600 rounded focus:ring-0 cursor-pointer"
                            />
                            <span>{f.value.toLowerCase() === 'yes' ? 'Checked (Yes)' : 'Unchecked (Off)'}</span>
                          </label>
                        ) : f.field_type === 'Choice' ? (
                          <select
                            value={f.value}
                            onChange={(e) => onUpdateFormFieldValue?.(f.name, e.target.value)}
                            className="w-full text-xs p-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 outline-none focus:border-purple-500"
                          >
                            {f.options.map((opt) => (
                              <option key={opt} value={opt}>
                                {opt}
                              </option>
                            ))}
                          </select>
                        ) : (
                          <input
                            type="text"
                            value={f.value}
                            onChange={(e) => onUpdateFormFieldValue?.(f.name, e.target.value)}
                            placeholder={f.alt_name || f.name}
                            className="w-full text-xs p-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 outline-none focus:border-purple-500"
                          />
                        )}
                      </div>

                      <div className="mt-2 pt-2 border-t border-neutral-200/60 dark:border-neutral-700/60 flex items-center justify-between text-[10px] text-neutral-400 font-mono">
                        <span>Pág. {f.page_number}</span>
                        <span>
                          {Math.round(f.bbox.width)}x{Math.round(f.bbox.height)} pt
                        </span>
                      </div>
                    </div>
                  );
                })}

                {/* Permanent Form Flattening Action */}
                {onFlattenForms && forms.length > 0 && (
                  <div className="pt-2">
                    <button
                      onClick={onFlattenForms}
                      className="w-full flex items-center justify-center gap-1.5 py-2 px-3 bg-purple-600 hover:bg-purple-700 text-white rounded-md text-xs font-semibold shadow-xs transition-colors"
                    >
                      <Layers size={13} />
                      <span>Flatten All Forms (Burn In-Place)</span>
                    </button>
                    <p className="mt-1 text-[10px] text-neutral-400 text-center">
                      Burns interactive fields into permanent vectors/text.
                    </p>
                  </div>
                )}
              </div>
            )}
          </>
        ) : activeTab === 'annots' ? (
          <div className="space-y-4">
            <div className="text-[11px] font-semibold text-neutral-400 uppercase px-1 flex items-center justify-between">
              <span>Page Annotations & Stamps</span>
              <span className="text-amber-500 font-mono">
                {annotations.filter((a) => a.page_number === pageNumber).length} Active
              </span>
            </div>

            {/* Quick Creation Actions Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
              <div className="font-semibold text-xs text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                <Highlighter size={13} className="text-amber-500" />
                <span>Text Markups & Stamps</span>
              </div>

              {/* Text Markup quick triggers */}
              <div className="grid grid-cols-3 gap-1.5">
                <button
                  onClick={() => onAddMarkup?.('Highlight')}
                  className="flex items-center justify-center gap-1 py-1.5 px-2 rounded border border-amber-300/80 dark:border-amber-700/80 bg-amber-50/80 dark:bg-amber-950/40 text-amber-900 dark:text-amber-200 text-[11px] font-medium hover:bg-amber-100 transition-colors cursor-pointer"
                >
                  <Highlighter size={12} />
                  <span>Highlight</span>
                </button>
                <button
                  onClick={() => onAddMarkup?.('Underline')}
                  className="flex items-center justify-center gap-1 py-1.5 px-2 rounded border border-blue-300/80 dark:border-blue-700/80 bg-blue-50/80 dark:bg-blue-950/40 text-blue-900 dark:text-blue-200 text-[11px] font-medium hover:bg-blue-100 transition-colors cursor-pointer"
                >
                  <Underline size={12} />
                  <span>Underline</span>
                </button>
                <button
                  onClick={() => onAddMarkup?.('StrikeOut')}
                  className="flex items-center justify-center gap-1 py-1.5 px-2 rounded border border-red-300/80 dark:border-red-700/80 bg-red-50/80 dark:bg-red-950/40 text-red-900 dark:text-red-200 text-[11px] font-medium hover:bg-red-100 transition-colors cursor-pointer"
                >
                  <span className="line-through text-xs font-bold">S</span>
                  <span>Strike</span>
                </button>
              </div>

              {/* Interactive Web Link Adder */}
              <div className="pt-2 border-t border-neutral-200 dark:border-neutral-700 space-y-1.5">
                <div className="flex items-center gap-1 text-[11px] font-medium text-neutral-700 dark:text-neutral-300">
                  <Link2 size={12} className="text-indigo-500" />
                  <span>Add Web Link:</span>
                </div>
                <div className="flex gap-1.5">
                  <input
                    type="url"
                    value={linkInputUrl}
                    onChange={(e) => setLinkInputUrl(e.target.value)}
                    placeholder="https://example.com"
                    className="flex-1 text-xs px-2 py-1 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-800 dark:text-neutral-200 outline-none focus:border-indigo-500"
                  />
                  <button
                    onClick={() => {
                      if (linkInputUrl && onAddLink) {
                        onAddLink(linkInputUrl);
                      }
                    }}
                    className="px-2.5 py-1 rounded bg-indigo-600 hover:bg-indigo-700 text-white text-xs font-semibold transition-colors cursor-pointer"
                  >
                    Add
                  </button>
                </div>
              </div>

              {/* Rubber Stamp Palette */}
              <div className="pt-2 border-t border-neutral-200 dark:border-neutral-700 space-y-1.5">
                <div className="flex items-center gap-1 text-[11px] font-medium text-neutral-700 dark:text-neutral-300">
                  <Award size={12} className="text-emerald-500" />
                  <span>Rubber Stamps:</span>
                </div>
                <div className="grid grid-cols-3 gap-1">
                  {['APPROVED', 'CONFIDENTIAL', 'DRAFT', 'REJECTED', 'FINAL', 'TOP SECRET'].map((stamp) => (
                    <button
                      key={stamp}
                      onClick={() => onAddStamp?.(stamp)}
                      className="py-1 px-1.5 rounded border border-neutral-200 dark:border-neutral-700 hover:border-emerald-500 text-[10px] font-bold text-neutral-700 dark:text-neutral-300 hover:text-emerald-600 transition-colors uppercase truncate text-center cursor-pointer"
                    >
                      {stamp}
                    </button>
                  ))}
                </div>
              </div>
            </div>

            {/* List of active annotations on current page */}
            <div className="space-y-2">
              {annotations
                .filter((a) => a.page_number === pageNumber)
                .map((a) => {
                  const isSelected = selectedAnnotationId === a.id;
                  return (
                    <div
                      key={a.id}
                      onClick={() => onSelectAnnotation?.(a.id)}
                      className={`p-2.5 rounded-lg border transition-all cursor-pointer ${
                        isSelected
                          ? 'border-amber-500 bg-amber-50/30 dark:bg-amber-950/20 ring-1 ring-amber-500/50'
                          : 'border-neutral-200 dark:border-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 bg-white dark:bg-neutral-800/40'
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <span className="flex items-center gap-1.5 text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                          {a.subtype === 'Highlight' && <Highlighter size={12} className="text-amber-500" />}
                          {a.subtype === 'Underline' && <Underline size={12} className="text-blue-500" />}
                          {a.subtype === 'Link' && <ExternalLink size={12} className="text-indigo-500" />}
                          {a.subtype === 'Stamp' && <Award size={12} className="text-emerald-500" />}
                          <span>{a.subtype} #{a.id}</span>
                        </span>
                        {onDeleteAnnotation && (
                          <button
                            onClick={(e) => {
                              e.stopPropagation();
                              onDeleteAnnotation(a.id);
                            }}
                            title="Delete Annotation"
                            className="p-1 text-neutral-400 hover:text-red-500 transition-colors cursor-pointer"
                          >
                            <Trash2 size={12} />
                          </button>
                        )}
                      </div>

                      {a.contents && (
                        <p className="mt-1 text-xs text-neutral-600 dark:text-neutral-400 line-clamp-1 italic">
                          "{a.contents}"
                        </p>
                      )}

                      {a.link_uri && (
                        <p className="mt-1 text-xs text-indigo-600 dark:text-indigo-400 line-clamp-1 font-mono">
                          {a.link_uri}
                        </p>
                      )}

                      {a.stamp_type && (
                        <p className="mt-1 text-xs font-bold text-emerald-600 dark:text-emerald-400">
                          Rubric: {a.stamp_type}
                        </p>
                      )}

                      <div className="mt-1 pt-1.5 border-t border-neutral-100 dark:border-neutral-700/60 flex items-center justify-between text-[10px] text-neutral-400 font-mono">
                        <span>
                          [{Math.round(a.bbox.min_x)}, {Math.round(a.bbox.min_y)}] - [{Math.round(a.bbox.max_x)}, {Math.round(a.bbox.max_y)}]
                        </span>
                        <span>{Math.round(a.bbox.width)}x{Math.round(a.bbox.height)}pt</span>
                      </div>
                    </div>
                  );
                })}
            </div>

            {/* Flatten Annotations Button */}
            {onFlattenAnnotations && annotations.filter((a) => a.page_number === pageNumber).length > 0 && (
              <div className="pt-2">
                <button
                  onClick={onFlattenAnnotations}
                  className="w-full flex items-center justify-center gap-1.5 py-2 px-3 bg-amber-600 hover:bg-amber-700 text-white rounded-md text-xs font-semibold shadow-xs transition-colors cursor-pointer"
                >
                  <Layers size={13} />
                  <span>Flatten Visual Annotations</span>
                </button>
                <p className="mt-1 text-[10px] text-neutral-400 text-center">
                  Bakes highlights, underlines & stamps into permanent page vector graphics.
                </p>
              </div>
            )}
          </div>
        ) : (
          <div className="space-y-4">
            <div className="text-[11px] font-semibold text-neutral-400 uppercase px-1">
              Document Assembly & Pages
            </div>

            {/* Current Page Geometry & Rotation Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-xs text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                  <RotateCw size={13} className="text-blue-500" />
                  Page {pageNumber} Orientation
                </span>
                <span className="font-mono text-[10px] px-2 py-0.5 rounded bg-blue-100 dark:bg-blue-950/60 text-blue-700 dark:text-blue-300 font-semibold">
                  {pageRotation}°
                </span>
              </div>

              <div className="grid grid-cols-3 gap-1.5">
                <button
                  onClick={() => onRotatePage?.(270)}
                  title="Rotate 90° CCW"
                  className="flex flex-col items-center justify-center p-2 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-700/60 text-neutral-700 dark:text-neutral-300 text-[11px] transition-colors cursor-pointer"
                >
                  <RotateCcw size={14} className="mb-1" />
                  <span>-90°</span>
                </button>
                <button
                  onClick={() => onRotatePage?.(90)}
                  title="Rotate 90° CW"
                  className="flex flex-col items-center justify-center p-2 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-700/60 text-neutral-700 dark:text-neutral-300 text-[11px] transition-colors cursor-pointer"
                >
                  <RotateCw size={14} className="mb-1" />
                  <span>+90°</span>
                </button>
                <button
                  onClick={() => onRotatePage?.(180)}
                  title="Rotate 180°"
                  className="flex flex-col items-center justify-center p-2 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 hover:bg-neutral-100 dark:hover:bg-neutral-700/60 text-neutral-700 dark:text-neutral-300 text-[11px] transition-colors cursor-pointer"
                >
                  <RefreshCw size={14} className="mb-1" />
                  <span>180°</span>
                </button>
              </div>

              {onRotateAllPages && (
                <button
                  onClick={() => onRotateAllPages(90)}
                  className="w-full flex items-center justify-center gap-1.5 py-1.5 px-2 bg-neutral-200 dark:bg-neutral-700 hover:bg-neutral-300 dark:hover:bg-neutral-600 text-neutral-800 dark:text-neutral-100 rounded text-xs font-medium transition-colors cursor-pointer"
                >
                  <RotateCw size={12} />
                  <span>Rotate All Pages (+90°)</span>
                </button>
              )}
            </div>

            {/* Split Document Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
              <div className="flex items-center gap-1.5 font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                <Scissors size={13} className="text-amber-500" />
                <span>Document Splitter</span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                Extract all pages into standalone single-page PDF documents.
              </p>
              <button
                onClick={onSplitDocument}
                className="w-full flex items-center justify-center gap-1.5 py-1.5 px-3 bg-amber-600 hover:bg-amber-700 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer"
              >
                <Scissors size={12} />
                <span>Split Document</span>
              </button>
            </div>

            {/* Merge Documents Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
              <div className="flex items-center gap-1.5 font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                <FileStack size={13} className="text-blue-500" />
                <span>Merge / Append PDF</span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                Concatenate another PDF file at the end of this document.
              </p>
              <button
                onClick={onTriggerMergeDocument}
                className="w-full flex items-center justify-center gap-1.5 py-1.5 px-3 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer"
              >
                <Plus size={12} />
                <span>Select & Append PDF...</span>
              </button>
            </div>

            {/* Delete Page Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
              <div className="flex items-center gap-1.5 font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                <Trash2 size={13} className="text-red-500" />
                <span>Delete Active Page</span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                Permanently purge Page {pageNumber} from document tree.
              </p>
              <button
                onClick={onDeleteCurrentPage}
                disabled={totalPages <= 1}
                className="w-full flex items-center justify-center gap-1.5 py-1.5 px-3 bg-red-600 hover:bg-red-700 disabled:opacity-40 disabled:hover:bg-red-600 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer"
              >
                <Trash2 size={12} />
                <span>Delete Page {pageNumber}</span>
              </button>
            </div>
          </div>
        )}

        {activeTab === 'watermark' && (
          <div className="space-y-4">
            {/* Dynamic Pagination & Bates Numbering Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                  <Stamp size={13} className="text-indigo-500" />
                  <span>Dynamic Foliado & Bates</span>
                </div>
                <span className="text-[10px] text-neutral-400 font-mono">ISO 32000 §8.4</span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                Inject dynamic headers, footers, and Bates numbering across pages.
              </p>

              {/* Template Format Chips */}
              <div className="space-y-1">
                <label className="text-[10px] font-medium text-neutral-500 uppercase">Format Template</label>
                <div className="flex flex-wrap gap-1">
                  {[
                    'Página {page} de {total}',
                    'Page {page} of {total}',
                    '- {page} -',
                    'DocRef-00{page}',
                  ].map((fmt) => (
                    <button
                      key={fmt}
                      type="button"
                      onClick={() => setPagFormat(fmt)}
                      className={`text-[9px] px-1.5 py-0.5 rounded border transition-colors cursor-pointer ${
                        pagFormat === fmt
                          ? 'border-indigo-500 bg-indigo-50 text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300 font-medium'
                          : 'border-neutral-200 dark:border-neutral-700 text-neutral-600 dark:text-neutral-400 hover:border-neutral-300'
                      }`}
                    >
                      {fmt}
                    </button>
                  ))}
                </div>
                <input
                  type="text"
                  value={pagFormat}
                  onChange={(e) => setPagFormat(e.target.value)}
                  className="w-full mt-1 text-xs px-2 py-1.5 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-mono focus:outline-hidden focus:ring-1 focus:ring-indigo-500"
                />
              </div>

              {/* Spatial Placement Grid */}
              <div className="space-y-1">
                <label className="text-[10px] font-medium text-neutral-500 uppercase">Position</label>
                <div className="grid grid-cols-3 gap-1 text-[10px]">
                  {[
                    { id: 'top_left', label: 'Top Left' },
                    { id: 'top_center', label: 'Top Center' },
                    { id: 'top_right', label: 'Top Right' },
                    { id: 'bottom_left', label: 'Btm Left' },
                    { id: 'bottom_center', label: 'Btm Center' },
                    { id: 'bottom_right', label: 'Btm Right' },
                  ].map((pos) => (
                    <button
                      key={pos.id}
                      type="button"
                      onClick={() => setPagPosition(pos.id as any)}
                      className={`py-1 px-1 text-center rounded border transition-colors cursor-pointer ${
                        pagPosition === pos.id
                          ? 'border-indigo-500 bg-indigo-50 text-indigo-700 dark:bg-indigo-950/40 dark:text-indigo-300 font-medium'
                          : 'border-neutral-200 dark:border-neutral-700 text-neutral-600 dark:text-neutral-400 hover:border-neutral-300'
                      }`}
                    >
                      {pos.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* Options: Font size, Margin */}
              <div className="grid grid-cols-2 gap-2 text-xs">
                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">Font Size (pt)</label>
                  <input
                    type="number"
                    min={6}
                    max={24}
                    value={pagFontSize}
                    onChange={(e) => setPagFontSize(Number(e.target.value))}
                    className="w-full mt-1 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200"
                  />
                </div>
                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">Margin (pt)</label>
                  <input
                    type="number"
                    min={10}
                    max={100}
                    value={pagMargin}
                    onChange={(e) => setPagMargin(Number(e.target.value))}
                    className="w-full mt-1 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200"
                  />
                </div>
              </div>

              <label className="flex items-center gap-2 text-xs text-neutral-700 dark:text-neutral-300 cursor-pointer pt-1">
                <input
                  type="checkbox"
                  checked={pagSkipFirst}
                  onChange={(e) => setPagSkipFirst(e.target.checked)}
                  className="rounded text-indigo-600 focus:ring-indigo-500 cursor-pointer"
                />
                <span>Skip First Page (Cover)</span>
              </label>

              <button
                type="button"
                onClick={() =>
                  onApplyPagination?.({
                    format: pagFormat,
                    position: pagPosition,
                    font_size: pagFontSize,
                    margin: pagMargin,
                    skip_first_page: pagSkipFirst,
                  })
                }
                className="w-full py-1.5 px-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
              >
                <Stamp size={12} />
                <span>Apply Foliado Across Pages</span>
              </button>
            </div>

            {/* Text Watermark Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                  <Award size={13} className="text-rose-500" />
                  <span>Text Watermark</span>
                </div>
                <span className="text-[10px] text-neutral-400 font-mono">ExtGState /ca</span>
              </div>

              {/* Text Presets */}
              <div className="space-y-1">
                <div className="flex flex-wrap gap-1">
                  {['CONFIDENCIAL', 'BORRADOR', 'DRAFT', 'ORIGINAL', 'COPIA'].map((t) => (
                    <button
                      key={t}
                      type="button"
                      onClick={() => setWmText(t)}
                      className={`text-[9px] px-1.5 py-0.5 rounded border transition-colors cursor-pointer ${
                        wmText === t
                          ? 'border-rose-500 bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300 font-semibold'
                          : 'border-neutral-200 dark:border-neutral-700 text-neutral-600 dark:text-neutral-400'
                      }`}
                    >
                      {t}
                    </button>
                  ))}
                </div>
                <input
                  type="text"
                  value={wmText}
                  onChange={(e) => setWmText(e.target.value)}
                  className="w-full mt-1 text-xs px-2 py-1.5 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-semibold focus:outline-hidden focus:ring-1 focus:ring-rose-500"
                />
              </div>

              {/* Depth Placement & Rotation */}
              <div className="grid grid-cols-2 gap-2 text-xs">
                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">Layer</label>
                  <div className="flex mt-1 rounded border border-neutral-200 dark:border-neutral-700 p-0.5 bg-neutral-100 dark:bg-neutral-800">
                    <button
                      type="button"
                      onClick={() => setWmPlacement('background')}
                      className={`flex-1 py-1 text-[10px] rounded transition-colors cursor-pointer ${
                        wmPlacement === 'background'
                          ? 'bg-white dark:bg-neutral-700 font-semibold shadow-xs text-neutral-900 dark:text-neutral-100'
                          : 'text-neutral-500'
                      }`}
                    >
                      Back
                    </button>
                    <button
                      type="button"
                      onClick={() => setWmPlacement('foreground')}
                      className={`flex-1 py-1 text-[10px] rounded transition-colors cursor-pointer ${
                        wmPlacement === 'foreground'
                          ? 'bg-white dark:bg-neutral-700 font-semibold shadow-xs text-neutral-900 dark:text-neutral-100'
                          : 'text-neutral-500'
                      }`}
                    >
                      Front
                    </button>
                  </div>
                </div>

                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">Rotation</label>
                  <select
                    value={wmRotation}
                    onChange={(e) => setWmRotation(Number(e.target.value))}
                    className="w-full mt-1 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 text-xs"
                  >
                    <option value={45}>45° Diagonal</option>
                    <option value={0}>0° Horizontal</option>
                    <option value={-45}>-45° Diagonal</option>
                    <option value={90}>90° Vertical</option>
                  </select>
                </div>
              </div>

              {/* Opacity & Font Size */}
              <div className="space-y-1">
                <div className="flex justify-between text-[11px] text-neutral-500">
                  <span>Opacity (Alpha):</span>
                  <span className="font-mono font-medium">{Math.round(wmOpacity * 100)}%</span>
                </div>
                <input
                  type="range"
                  min={5}
                  max={60}
                  value={Math.round(wmOpacity * 100)}
                  onChange={(e) => setWmOpacity(Number(e.target.value) / 100)}
                  className="w-full accent-rose-600"
                />
              </div>

              {/* Target Scope */}
              <div className="flex items-center justify-between text-xs text-neutral-600 dark:text-neutral-400">
                <span>Apply to:</span>
                <div className="flex gap-2">
                  <label className="flex items-center gap-1 cursor-pointer">
                    <input
                      type="radio"
                      name="wmTarget"
                      checked={wmTargetAll}
                      onChange={() => setWmTargetAll(true)}
                    />
                    <span>All Pages</span>
                  </label>
                  <label className="flex items-center gap-1 cursor-pointer">
                    <input
                      type="radio"
                      name="wmTarget"
                      checked={!wmTargetAll}
                      onChange={() => setWmTargetAll(false)}
                    />
                    <span>Page {pageNumber}</span>
                  </label>
                </div>
              </div>

              <button
                type="button"
                onClick={() =>
                  onApplyTextWatermark?.({
                    text: wmText,
                    font_size: wmFontSize,
                    opacity: wmOpacity,
                    rotation_degrees: wmRotation,
                    placement: wmPlacement,
                    page_indices: wmTargetAll ? undefined : [pageNumber - 1],
                  })
                }
                className="w-full py-1.5 px-3 bg-rose-600 hover:bg-rose-700 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
              >
                <Award size={12} />
                <span>Apply Text Watermark</span>
              </button>
            </div>

            {/* Image Watermark Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                  <ImageIcon size={13} className="text-emerald-500" />
                  <span>Image Watermark / Logo</span>
                </div>
                <span className="text-[10px] text-neutral-400 font-mono">PNG / JPEG</span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                Embed a semi-transparent company logo or official seal.
              </p>
              <button
                type="button"
                onClick={onTriggerImageWatermark}
                className="w-full py-1.5 px-3 bg-emerald-600 hover:bg-emerald-700 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
              >
                <Plus size={12} />
                <span>Upload & Apply Logo Watermark...</span>
              </button>
            </div>
          </div>
        )}

        {/* Tab 7: Redaction & Sanitization Panel */}
        {activeTab === 'redact' && (
          <div className="space-y-4">
            {/* Warning Banner */}
            <div className="p-3 rounded-lg border border-rose-300 dark:border-rose-900/60 bg-rose-50/80 dark:bg-rose-950/30 text-rose-900 dark:text-rose-200 space-y-1.5">
              <div className="flex items-center gap-1.5 font-semibold text-xs text-rose-700 dark:text-rose-300">
                <ShieldAlert size={14} className="text-rose-600 dark:text-rose-400 shrink-0" />
                <span>Excisión de glifos</span>
              </div>
              <p className="text-[11px] leading-relaxed text-rose-800 dark:text-rose-300/90">
                Quita los glifos que cruzan la zona y dibuja un recuadro opaco. No borra adjuntos, la estructura ni las apariencias de formulario. Los metadatos del documento se quitan solo si se pide el barrido.
              </p>
            </div>

            {/* Sub-mode selector */}
            <div className="grid grid-cols-3 gap-1 p-0.5 bg-neutral-100 dark:bg-neutral-800 rounded-lg">
              <button
                type="button"
                onClick={() => setRedactSubMode('pattern')}
                className={`py-1 text-[10px] font-medium rounded-md transition-all cursor-pointer ${
                  redactSubMode === 'pattern'
                    ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs font-semibold'
                    : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
                }`}
              >
                Patrón PII
              </button>
              <button
                type="button"
                onClick={() => setRedactSubMode('text')}
                className={`py-1 text-[10px] font-medium rounded-md transition-all cursor-pointer ${
                  redactSubMode === 'text'
                    ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs font-semibold'
                    : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
                }`}
              >
                Texto Exacto
              </button>
              <button
                type="button"
                onClick={() => setRedactSubMode('region')}
                className={`py-1 text-[10px] font-medium rounded-md transition-all cursor-pointer ${
                  redactSubMode === 'region'
                    ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs font-semibold'
                    : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
                }`}
              >
                Coordenadas
              </button>
            </div>

            {/* Mode 1: PII Pattern Presets */}
            {redactSubMode === 'pattern' && (
              <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2.5">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                    Tipo de Dato Sensible
                  </span>
                  <span className="text-[10px] text-neutral-400 font-mono">Regex Scan</span>
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  {[
                    { id: 'email', label: 'Emails / Correo' },
                    { id: 'phone', label: 'Teléfonos (+52 / Int)' },
                    { id: 'rfc', label: 'RFC Mexicano' },
                    { id: 'curp', label: 'CURP Mexicano' },
                    { id: 'credit_card', label: 'Tarjetas (Luhn)' },
                    { id: 'ssn', label: 'SSN (Seguro Social)' },
                  ].map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => setRedactPatternType(p.id as any)}
                      className={`text-left text-[10px] px-2 py-1.5 rounded border transition-colors cursor-pointer ${
                        redactPatternType === p.id
                          ? 'border-rose-500 bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300 font-semibold'
                          : 'border-neutral-200 dark:border-neutral-700 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800'
                      }`}
                    >
                      {p.label}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {/* Mode 2: Exact Text Query */}
            {redactSubMode === 'text' && (
              <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                    Buscar y Eliminar Texto
                  </span>
                  <span className="text-[10px] text-neutral-400 font-mono">AST Search</span>
                </div>
                <input
                  type="text"
                  value={redactQueryText}
                  onChange={(e) => setRedactQueryText(e.target.value)}
                  placeholder="Ej: Juan Pérez, 1234-5678..."
                  className="w-full text-xs px-2.5 py-1.5 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 focus:outline-hidden focus:ring-1 focus:ring-rose-500"
                />
                <p className="text-[10px] text-neutral-400">
                  Búsqueda insensible a mayúsculas/minúsculas. Elimina los glifos correspondientes del flujo de texto.
                </p>
              </div>
            )}

            {/* Mode 3: Manual Bounding Box Coordinates */}
            {redactSubMode === 'region' && (
              <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                    Caja Delimitadora (Puntos PDF)
                  </span>
                  <span className="text-[10px] text-neutral-400 font-mono">72 pt = 1 pulg</span>
                </div>
                <div className="grid grid-cols-2 gap-2 text-xs">
                  <div>
                    <label className="text-[10px] text-neutral-500 font-medium">Min X</label>
                    <input
                      type="number"
                      value={redactBoxMinX}
                      onChange={(e) => setRedactBoxMinX(Number(e.target.value))}
                      className="w-full mt-0.5 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-mono text-xs"
                    />
                  </div>
                  <div>
                    <label className="text-[10px] text-neutral-500 font-medium">Min Y</label>
                    <input
                      type="number"
                      value={redactBoxMinY}
                      onChange={(e) => setRedactBoxMinY(Number(e.target.value))}
                      className="w-full mt-0.5 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-mono text-xs"
                    />
                  </div>
                  <div>
                    <label className="text-[10px] text-neutral-500 font-medium">Max X</label>
                    <input
                      type="number"
                      value={redactBoxMaxX}
                      onChange={(e) => setRedactBoxMaxX(Number(e.target.value))}
                      className="w-full mt-0.5 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-mono text-xs"
                    />
                  </div>
                  <div>
                    <label className="text-[10px] text-neutral-500 font-medium">Max Y</label>
                    <input
                      type="number"
                      value={redactBoxMaxY}
                      onChange={(e) => setRedactBoxMaxY(Number(e.target.value))}
                      className="w-full mt-0.5 px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-mono text-xs"
                    />
                  </div>
                </div>
              </div>
            )}

            {/* Redaction Options Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2.5">
              <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                Opciones de Censura
              </span>

              {/* Overlay Text */}
              <div>
                <label className="text-[10px] font-medium text-neutral-500 uppercase">
                  Etiqueta Superpuesta (Opcional)
                </label>
                <div className="flex gap-1 mt-1">
                  <input
                    type="text"
                    value={redactOverlayLabel}
                    onChange={(e) => setRedactOverlayLabel(e.target.value)}
                    placeholder="Ej: [REDACTADO], [CENSURADO]"
                    className="flex-1 text-xs px-2 py-1 border border-neutral-300 dark:border-neutral-700 rounded bg-white dark:bg-neutral-900 text-neutral-800 dark:text-neutral-200 font-mono"
                  />
                  <button
                    type="button"
                    onClick={() => setRedactOverlayLabel('')}
                    className="px-2 py-1 text-[10px] border border-neutral-300 dark:border-neutral-700 rounded hover:bg-neutral-100 dark:hover:bg-neutral-800 text-neutral-600 dark:text-neutral-400 cursor-pointer"
                    title="Caja negra pura"
                  >
                    Negro Puro
                  </button>
                </div>
              </div>

              {/* Scope */}
              {redactSubMode !== 'region' && (
                <div className="flex items-center justify-between text-xs text-neutral-600 dark:text-neutral-400 pt-1">
                  <span className="text-[11px]">Alcance:</span>
                  <div className="flex gap-2">
                    <label className="flex items-center gap-1 cursor-pointer text-[11px]">
                      <input
                        type="radio"
                        name="redactScope"
                        checked={redactTargetAll}
                        onChange={() => setRedactTargetAll(true)}
                      />
                      <span>Todo ({totalPages} págs)</span>
                    </label>
                    <label className="flex items-center gap-1 cursor-pointer text-[11px]">
                      <input
                        type="radio"
                        name="redactScope"
                        checked={!redactTargetAll}
                        onChange={() => setRedactTargetAll(false)}
                      />
                      <span>Pág {pageNumber}</span>
                    </label>
                  </div>
                </div>
              )}

              {/* Annotation Pruning & Metadata Scrubbing Checkboxes */}
              <div className="space-y-1.5 pt-1">
                <label className="flex items-start gap-1.5 cursor-pointer text-[11px] text-neutral-700 dark:text-neutral-300">
                  <input
                    type="checkbox"
                    checked={redactPruneAnnotations}
                    onChange={(e) => setRedactPruneAnnotations(e.target.checked)}
                    className="mt-0.5 rounded text-rose-600"
                  />
                  <span>Podar anotaciones (/Link, /Highlight) en zonas censuradas</span>
                </label>

                {redactSubMode === 'pattern' && (
                  <label className="flex items-start gap-1.5 cursor-pointer text-[11px] text-neutral-700 dark:text-neutral-300">
                    <input
                      type="checkbox"
                      checked={redactScrubMetadata}
                      onChange={(e) => setRedactScrubMetadata(e.target.checked)}
                      className="mt-0.5 rounded text-rose-600"
                    />
                    <span>Higienizar metadatos del documento (/Info, XMP)</span>
                  </label>
                )}
              </div>

              {/* Execute Redaction Button */}
              <button
                type="button"
                onClick={() => {
                  if (redactSubMode === 'pattern') {
                    onRedactPattern?.({
                      pattern_type: redactPatternType,
                      page_numbers: redactTargetAll ? undefined : [pageNumber],
                      overlay_text: redactOverlayLabel.trim() || undefined,
                      prune_annotations: redactPruneAnnotations,
                      scrub_metadata: redactScrubMetadata,
                    });
                  } else if (redactSubMode === 'text') {
                    if (!redactQueryText.trim()) {
                      alert('Por favor ingrese el texto a censurar.');
                      return;
                    }
                    onRedactText?.({
                      query: redactQueryText.trim(),
                      case_sensitive: false,
                      page_numbers: redactTargetAll ? undefined : [pageNumber],
                      overlay_text: redactOverlayLabel.trim() || undefined,
                      prune_annotations: redactPruneAnnotations,
                    });
                  } else if (redactSubMode === 'region') {
                    onRedactRegions?.({
                      page_number: pageNumber,
                      regions: [
                        {
                          min_x: redactBoxMinX,
                          min_y: redactBoxMinY,
                          max_x: redactBoxMaxX,
                          max_y: redactBoxMaxY,
                        },
                      ],
                      overlay_text: redactOverlayLabel.trim() || undefined,
                      prune_annotations: redactPruneAnnotations,
                    });
                  }
                }}
                className="w-full mt-2 py-2 px-3 bg-rose-600 hover:bg-rose-700 text-white rounded text-xs font-semibold transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
              >
                <EyeOff size={13} />
                <span>Aplicar Censura Quirúrgica</span>
              </button>
            </div>

            {/* Standalone Metadata Sanitizer Card */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                  Higienización de Metadatos
                </span>
                <span className="text-[10px] text-neutral-400 font-mono">/Info + XMP</span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400">
                Purga autor, creador, software, fechas y streams de metadatos XML XMP para evitar fugas de privacidad.
              </p>
              <button
                type="button"
                onClick={() => onSanitizeDocument?.(true)}
                className="w-full py-1.5 px-3 bg-neutral-800 hover:bg-neutral-900 dark:bg-neutral-700 dark:hover:bg-neutral-600 text-white rounded text-xs font-medium transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
              >
                <ShieldAlert size={12} className="text-amber-400" />
                <span>Purgar Metadatos del Documento</span>
              </button>
            </div>
          </div>
        )}

        {activeTab === 'security' && (
          <div className="space-y-4">
            {/* Header & Status Banner */}
            <div className="flex items-center justify-between px-1">
              <span className="text-[11px] font-semibold text-neutral-400 uppercase tracking-wider">
                Seguridad y Firmas
              </span>
              <span className="text-[10px] font-mono text-neutral-400">
                ISO 32000 §7.6 & §12.8
              </span>
            </div>

            {/* Status Card */}
            <div className={`p-3 rounded-lg border ${
              isEncrypted
                ? 'border-emerald-500/50 bg-emerald-50/40 dark:bg-emerald-950/20'
                : 'border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30'
            }`}>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {isEncrypted ? (
                    <Lock size={15} className="text-emerald-600 dark:text-emerald-400" />
                  ) : (
                    <Unlock size={15} className="text-neutral-500 dark:text-neutral-400" />
                  )}
                  <span className="text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                    {isEncrypted ? 'Cifrado AES-128 Activo' : 'Sin Cifrado (Abierto)'}
                  </span>
                </div>
                <span className={`text-[10px] px-1.5 py-0.5 rounded font-mono font-medium ${
                  isEncrypted
                    ? 'bg-emerald-100 dark:bg-emerald-900/60 text-emerald-700 dark:text-emerald-300'
                    : 'bg-neutral-200 dark:bg-neutral-700 text-neutral-600 dark:text-neutral-300'
                }`}>
                  {isEncrypted ? 'Rev 4 (AES-CBC)' : 'Desprotegido'}
                </span>
              </div>
              <p className="mt-1 text-[11px] text-neutral-500 dark:text-neutral-400 leading-normal">
                {isEncrypted
                  ? 'El documento está protegido con cifrado de flujo y diccionario /Encrypt activo.'
                  : 'Cifre el documento para restringir la visualización, copia, impresión y edición.'}
              </p>
            </div>

            {/* Encryption & Decryption Controls */}
            {isEncrypted ? (
              <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                  <Key size={13} className="text-amber-500" />
                  <span>Descifrar Documento</span>
                </div>
                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">
                    Contraseña de Usuario o Admin
                  </label>
                  <input
                    type="password"
                    value={secDecryptPass}
                    onChange={(e) => setSecDecryptPass(e.target.value)}
                    placeholder="Ingrese contraseña..."
                    className="w-full mt-1 px-2.5 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200 focus:outline-hidden focus:ring-1 focus:ring-amber-500"
                  />
                </div>
                <button
                  type="button"
                  onClick={() => {
                    if (!secDecryptPass) {
                      alert('Ingrese la contraseña para descifrar.');
                      return;
                    }
                    onDecryptDocument?.(secDecryptPass);
                    setSecDecryptPass('');
                  }}
                  className="w-full py-2 px-3 bg-neutral-800 hover:bg-neutral-900 dark:bg-neutral-700 dark:hover:bg-neutral-600 text-white rounded text-xs font-semibold transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
                >
                  <Unlock size={12} />
                  <span>Descifrar y Quitar Restricciones</span>
                </button>
              </div>
            ) : (
              <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                  <Lock size={13} className="text-amber-500" />
                  <span>Cifrar con AES-128</span>
                </div>
                <div className="space-y-2">
                  <div>
                    <label className="text-[10px] font-medium text-neutral-500 uppercase">
                      Contraseña de Apertura (Usuario)
                    </label>
                    <input
                      type="password"
                      value={secUserPass}
                      onChange={(e) => setSecUserPass(e.target.value)}
                      placeholder="Opcional (dejar vacío si es abierto)"
                      className="w-full mt-0.5 px-2.5 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200 focus:outline-hidden focus:ring-1 focus:ring-amber-500"
                    />
                  </div>
                  <div>
                    <label className="text-[10px] font-medium text-neutral-500 uppercase">
                      Contraseña de Administración (Propietario)
                    </label>
                    <input
                      type="password"
                      value={secOwnerPass}
                      onChange={(e) => setSecOwnerPass(e.target.value)}
                      placeholder="Requerida"
                      className="w-full mt-0.5 px-2.5 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200 focus:outline-hidden focus:ring-1 focus:ring-amber-500"
                    />
                  </div>
                </div>

                {/* Permissions Bitmask Checkboxes */}
                <div className="space-y-1.5 pt-1 border-t border-neutral-200/60 dark:border-neutral-800">
                  <span className="text-[10px] font-semibold text-neutral-400 uppercase">
                    Bits /P almacenados, este proceso no los aplica
                  </span>
                  <div className="grid grid-cols-1 gap-1 text-[11px] text-neutral-600 dark:text-neutral-300">
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermPrintHigh}
                        onChange={(e) => setSecPermPrintHigh(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Impresión en Alta Resolución</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermModifyContents}
                        onChange={(e) => setSecPermModifyContents(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Modificación de Contenido</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermCopyExtract}
                        onChange={(e) => setSecPermCopyExtract(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Copia y Extracción de Texto</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermModifyAnnots}
                        onChange={(e) => setSecPermModifyAnnots(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Anotaciones y Comentarios</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermFillForms}
                        onChange={(e) => setSecPermFillForms(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Llenado de Formularios Interactivos</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermAccessibility}
                        onChange={(e) => setSecPermAccessibility(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Extracción para Accesibilidad</span>
                    </label>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={secPermAssemble}
                        onChange={(e) => setSecPermAssemble(e.target.checked)}
                        className="rounded text-amber-600"
                      />
                      <span>Ensamblado de Páginas</span>
                    </label>
                  </div>
                </div>

                <button
                  type="button"
                  disabled={secOwnerPass.length === 0}
                  onClick={() => {
                    if (secOwnerPass.length === 0) {
                      return;
                    }
                    onEncryptDocument?.({
                      user_password: secUserPass,
                      owner_password: secOwnerPass,
                      permissions: {
                        print_low_res: true,
                        print_high_res: secPermPrintHigh,
                        modify_contents: secPermModifyContents,
                        copy_extract: secPermCopyExtract,
                        modify_annotations: secPermModifyAnnots,
                        fill_forms: secPermFillForms,
                        accessibility_extract: secPermAccessibility,
                        assemble_document: secPermAssemble,
                      },
                      encrypt_metadata: true,
                    });
                  }}
                  className="w-full py-2 px-3 bg-amber-600 hover:bg-amber-700 disabled:opacity-50 disabled:cursor-not-allowed text-white rounded text-xs font-semibold transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
                >
                  <Lock size={12} />
                  <span>Aplicar Cifrado AES-128</span>
                </button>
              </div>
            )}

            {/* Digital Signatures Section */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 text-xs font-semibold text-neutral-800 dark:text-neutral-200">
                  <FileCheck size={13} className="text-blue-500" />
                  <span>Firma Digital & Atestación</span>
                </div>
                <span className="text-[10px] text-neutral-400 font-mono">
                  {sigFormat === 'attestation' ? 'ByteRange SHA-256' : 'PKCS#7 Detached'}
                </span>
              </div>

              <div className="space-y-2">
                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">
                    Tipo de Firma
                  </label>
                  <select
                    value={sigFormat}
                    onChange={(e) => {
                      const val = e.target.value as 'attestation' | 'pkcs12' | 'pem';
                      setSigFormat(val);
                    }}
                    className="w-full mt-0.5 px-2 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200"
                  >
                    <option value="attestation">Atestación SHA-256 (sin certificado)</option>
                    <option value="pkcs12">Certificado PKCS#12 / PFX (.p12 / .pfx)</option>
                    <option value="pem">Certificado y Llave Privada PEM (X.509)</option>
                  </select>
                </div>

                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">
                    Nombre del Firmante / Entidad
                  </label>
                  <input
                    type="text"
                    value={sigName}
                    onChange={(e) => setSigName(e.target.value)}
                    placeholder="Ej. Lic. Roberto Garduño"
                    className="w-full mt-0.5 px-2.5 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200 focus:outline-hidden focus:ring-1 focus:ring-blue-500"
                  />
                </div>
                <div>
                  <label className="text-[10px] font-medium text-neutral-500 uppercase">
                    Motivo / Razón Legal
                  </label>
                  <input
                    type="text"
                    value={sigReason}
                    onChange={(e) => setSigReason(e.target.value)}
                    placeholder="Ej. Aprobación y Certificación Legal"
                    className="w-full mt-0.5 px-2.5 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200 focus:outline-hidden focus:ring-1 focus:ring-blue-500"
                  />
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label className="text-[10px] font-medium text-neutral-500 uppercase">
                      Lugar / Jurisdicción
                    </label>
                    <input
                      type="text"
                      value={sigLocation}
                      onChange={(e) => setSigLocation(e.target.value)}
                      placeholder="CDMX, MX"
                      className="w-full mt-0.5 px-2 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200"
                    />
                  </div>
                  <div>
                    <label className="text-[10px] font-medium text-neutral-500 uppercase">
                      Página Destino
                    </label>
                    <input
                      type="number"
                      min={1}
                      max={totalPages}
                      value={sigPage}
                      onChange={(e) => setSigPage(parseInt(e.target.value) || 1)}
                      className="w-full mt-0.5 px-2 py-1.5 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200"
                    />
                  </div>
                </div>

                {sigFormat === 'pkcs12' && (
                  <div className="p-2.5 bg-blue-50/50 dark:bg-blue-950/20 border border-blue-200 dark:border-blue-900/40 rounded-lg space-y-2">
                    <div>
                      <label className="text-[10px] font-semibold text-neutral-600 dark:text-neutral-400 block mb-1">
                        Archivo de Certificado (.p12 / .pfx)
                      </label>
                      <input
                        type="file"
                        accept=".p12,.pfx"
                        onChange={(e) => {
                          const file = e.target.files?.[0];
                          if (!file) return;
                          setSigPkcs12Filename(file.name);
                          const reader = new FileReader();
                          reader.onload = () => {
                            const result = reader.result as string;
                            const b64 = result.includes(',') ? result.split(',')[1] : result;
                            setSigPkcs12Base64(b64);
                          };
                          reader.readAsDataURL(file);
                        }}
                        className="text-[11px] text-neutral-600 dark:text-neutral-400 file:mr-2 file:py-1 file:px-2 file:rounded file:border-0 file:text-[10px] file:bg-blue-600 file:text-white"
                      />
                      {sigPkcs12Filename && (
                        <div className="mt-1 text-[9px] text-blue-600 dark:text-blue-400 font-mono">
                          {sigPkcs12Filename} cargado ({Math.round(sigPkcs12Base64.length * 0.75)} bytes)
                        </div>
                      )}
                    </div>
                    <div>
                      <label className="text-[10px] font-semibold text-neutral-600 dark:text-neutral-400 block">
                        Contraseña del Certificado
                      </label>
                      <input
                        type="password"
                        value={sigPkcs12Password}
                        onChange={(e) => setSigPkcs12Password(e.target.value)}
                        placeholder="Contraseña del archivo .p12"
                        className="w-full mt-0.5 px-2 py-1 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded"
                      />
                    </div>
                  </div>
                )}

                {sigFormat === 'pem' && (
                  <div className="p-2.5 bg-blue-50/50 dark:bg-blue-950/20 border border-blue-200 dark:border-blue-900/40 rounded-lg space-y-2">
                    <div>
                      <label className="text-[10px] font-semibold text-neutral-600 dark:text-neutral-400 block">
                        Certificado PEM (X.509)
                      </label>
                      <textarea
                        rows={2}
                        value={sigCertPem}
                        onChange={(e) => setSigCertPem(e.target.value)}
                        placeholder="-----BEGIN CERTIFICATE----- ... -----END CERTIFICATE-----"
                        className="w-full mt-0.5 p-1.5 text-[10px] font-mono bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded resize-none"
                      />
                    </div>
                    <div>
                      <label className="text-[10px] font-semibold text-neutral-600 dark:text-neutral-400 block">
                        Clave Privada PEM (PKCS#8 / PKCS#1)
                      </label>
                      <textarea
                        rows={2}
                        value={sigKeyPem}
                        onChange={(e) => setSigKeyPem(e.target.value)}
                        placeholder="-----BEGIN PRIVATE KEY----- ... -----END PRIVATE KEY-----"
                        className="w-full mt-0.5 p-1.5 text-[10px] font-mono bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded resize-none"
                      />
                    </div>
                  </div>
                )}

                {/* TSA Time-Stamp Authority (RFC 3161) */}
                <div className="pt-1">
                  <label className="text-[10px] font-medium text-neutral-500 uppercase flex items-center justify-between">
                    <span>Servidor de Sellado de Tiempo TSA (RFC 3161)</span>
                    <span className="text-[9px] text-neutral-400 lowercase">opcional</span>
                  </label>
                  <input
                    type="url"
                    value={sigTsaUrl}
                    onChange={(e) => setSigTsaUrl(e.target.value)}
                    placeholder="https://freetsa.org/tsr o https://timestamp.digicert.com"
                    className="w-full mt-0.5 px-2 py-1 text-xs bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-700 rounded text-neutral-800 dark:text-neutral-200 placeholder:text-neutral-400"
                  />
                  <p className="text-[9px] text-neutral-400 mt-0.5">
                    Genera una marca de tiempo criptográfica inmutable sobre el valor de la firma.
                  </p>
                </div>
              </div>

              <button
                type="button"
                onClick={() => {
                  if (!sigName.trim()) {
                    alert('Ingrese el nombre del firmante.');
                    return;
                  }
                  const payload: SignDocumentPayload = {
                    signer_name: sigName.trim(),
                    reason: sigReason.trim(),
                    location: sigLocation.trim(),
                    page_number: sigPage,
                    rect: [72.0, 72.0, 272.0, 142.0],
                  };
                  if (sigFormat === 'pkcs12' && sigPkcs12Base64) {
                    payload.pkcs12_base64 = sigPkcs12Base64;
                    payload.pkcs12_password = sigPkcs12Password;
                  } else if (sigFormat === 'pem' && sigCertPem.trim() && sigKeyPem.trim()) {
                    payload.certificate_pem = sigCertPem.trim();
                    payload.private_key_pem = sigKeyPem.trim();
                  }
                  if (sigTsaUrl.trim()) {
                    payload.tsa_url = sigTsaUrl.trim();
                  }
                  onSignDocument?.(payload);
                }}
                className="w-full py-2 px-3 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs font-semibold transition-colors shadow-xs cursor-pointer flex items-center justify-center gap-1.5"
              >
                <CheckCircle2 size={13} />
                <span>
                  {sigFormat === 'attestation'
                    ? 'Estampar atestación SHA-256'
                    : 'Estampar Firma PKCS#7 (adbe.pkcs7.detached)'}
                </span>
              </button>
            </div>

            {/* Signatures List */}
            <div className="space-y-2">
              <div className="flex items-center justify-between px-1">
                <span className="text-[11px] font-semibold text-neutral-400 uppercase">
                  Atestaciones ({signatures.length})
                </span>
                <span className="text-[10px] font-mono text-neutral-400">ByteRange</span>
              </div>

              {signatures.length === 0 ? (
                <div className="p-3 rounded-lg border border-dashed border-neutral-200 dark:border-neutral-800 text-center text-xs text-neutral-400">
                  No hay firmas registradas en este documento.
                </div>
              ) : (
                signatures.map((sig, idx) => (
                  <div
                    key={idx}
                    className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 shadow-xs space-y-1.5"
                  >
                    <div className="flex items-center justify-between">
                      <span className="font-semibold text-xs text-neutral-800 dark:text-neutral-200">
                        {sig.signer_name}
                      </span>
                      <span className={`text-[9px] px-1.5 py-0.5 rounded font-mono font-medium flex items-center gap-1 ${
                        sig.byte_range_valid
                          ? 'bg-emerald-100 dark:bg-emerald-900/60 text-emerald-700 dark:text-emerald-300'
                          : 'bg-rose-100 dark:bg-rose-900/60 text-rose-700 dark:text-rose-300'
                      }`}>
                        <CheckCircle2 size={10} />
                        {sig.byte_range_valid ? 'Íntegra' : 'No coincide'}
                      </span>
                    </div>
                    <div className="text-[11px] text-neutral-600 dark:text-neutral-400 space-y-0.5">
                      <div><strong className="text-neutral-500 font-medium">Motivo:</strong> {sig.reason || 'Sin motivo'}</div>
                      <div><strong className="text-neutral-500 font-medium">Lugar:</strong> {sig.location || 'N/A'}</div>
                      <div><strong className="text-neutral-500 font-medium">Página:</strong> {sig.page_number}</div>
                      <div className="font-mono text-[10px] text-neutral-400 truncate">
                        Fmt: {sig.sub_filter}
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        )}

        {activeTab === 'tables' && (
          <div className="space-y-4">
            {/* Header info */}
            <div className="p-3 bg-neutral-50 dark:bg-neutral-800/40 rounded-lg border border-neutral-200 dark:border-neutral-800 space-y-1">
              <div className="flex items-center justify-between">
                <span className="font-semibold text-xs text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                  <TableIcon size={13} className="text-emerald-500" />
                  Extracción de Tablas
                </span>
                <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-emerald-100 dark:bg-emerald-950 text-emerald-700 dark:text-emerald-300 font-semibold">
                  ISO 32000 §14.8.4
                </span>
              </div>
              <p className="text-[10px] text-neutral-500 leading-tight">
                Reconstrucción vectorial por retícula (lattice) y flujo de texto semántico.
              </p>
            </div>

            {/* Tables count / list */}
            <div className="space-y-2">
              <div className="flex items-center justify-between px-1">
                <span className="text-[11px] font-semibold text-neutral-400 uppercase">
                  Tablas Detectadas ({tables.length})
                </span>
                <span className="text-[10px] font-mono text-neutral-400">Página {pageNumber}</span>
              </div>

              {tables.length === 0 ? (
                <div className="p-4 rounded-lg border border-dashed border-neutral-200 dark:border-neutral-800 text-center space-y-1.5 text-neutral-400">
                  <TableIcon size={24} className="mx-auto text-neutral-300 dark:text-neutral-600" />
                  <div className="text-xs font-medium">No se detectaron tablas</div>
                  <div className="text-[10px] text-neutral-500">
                    No hay rejillas vectoriales ni patrones tabulares en esta página.
                  </div>
                </div>
              ) : (
                tables.map((table) => {
                  const isSelected = selectedTableIdx === table.table_idx;
                  return (
                    <div
                      key={table.table_idx}
                      onClick={() => onSelectTable?.(table.table_idx)}
                      className={`p-3 rounded-lg border cursor-pointer transition-all ${
                        isSelected
                          ? 'border-emerald-500 bg-emerald-50/40 dark:bg-emerald-950/20 shadow-xs'
                          : 'border-neutral-200 dark:border-neutral-800 hover:border-neutral-300 dark:hover:border-neutral-700 bg-neutral-50/50 dark:bg-neutral-800/30'
                      }`}
                    >
                      <div className="flex items-center justify-between mb-1.5">
                        <span className="font-semibold text-xs text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                          <TableIcon size={12} className="text-emerald-500" />
                          Tabla #{table.table_idx + 1}
                        </span>
                        <div className="flex items-center gap-1">
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-neutral-200 dark:bg-neutral-700 text-neutral-700 dark:text-neutral-300 font-mono font-medium">
                            {table.row_count} × {table.col_count}
                          </span>
                          <span className="text-[9px] px-1.5 py-0.5 rounded bg-emerald-100 dark:bg-emerald-900/60 text-emerald-700 dark:text-emerald-300 font-mono">
                            {table.cells.length} celdas
                          </span>
                        </div>
                      </div>

                      <div className="text-[10px] text-neutral-500 font-mono mb-2">
                        BBox: [{table.bbox.min_x.toFixed(0)}, {table.bbox.min_y.toFixed(0)}, {table.bbox.max_x.toFixed(0)}, {table.bbox.max_y.toFixed(0)}]
                      </div>

                      {/* Mini table preview */}
                      <div className="max-h-36 overflow-auto border border-neutral-200 dark:border-neutral-800 rounded bg-white dark:bg-neutral-900 text-[10px]">
                        <table className="w-full border-collapse">
                          {table.headers && table.headers.length > 0 && (
                            <thead>
                              <tr className="bg-neutral-100 dark:bg-neutral-800 border-b border-neutral-200 dark:border-neutral-700">
                                {table.headers.map((h, hIdx) => (
                                  <th
                                    key={hIdx}
                                    className="p-1 text-left font-semibold text-neutral-700 dark:text-neutral-300 border-r border-neutral-200 dark:border-neutral-800 last:border-r-0 truncate max-w-[80px]"
                                    title={h}
                                  >
                                    {h || `Col ${hIdx + 1}`}
                                  </th>
                                ))}
                              </tr>
                            </thead>
                          )}
                          <tbody>
                            {table.rows.slice(0, 5).map((row, rIdx) => (
                              <tr
                                key={rIdx}
                                className="border-b border-neutral-100 dark:border-neutral-800 last:border-b-0 hover:bg-neutral-50 dark:hover:bg-neutral-800/40"
                              >
                                {row.map((val, cIdx) => (
                                  <td
                                    key={cIdx}
                                    className="p-1 border-r border-neutral-100 dark:border-neutral-800 last:border-r-0 text-neutral-600 dark:text-neutral-400 truncate max-w-[80px]"
                                    title={val}
                                  >
                                    {val || '-'}
                                  </td>
                                ))}
                              </tr>
                            ))}
                          </tbody>
                        </table>
                        {table.rows.length > 5 && (
                          <div className="text-[9px] text-neutral-400 text-center py-0.5 bg-neutral-50 dark:bg-neutral-800/30">
                            +{table.rows.length - 5} filas adicionales...
                          </div>
                        )}
                      </div>

                      {/* Export & Download Controls */}
                      {isSelected && (
                        <div className="mt-3 pt-2.5 border-t border-neutral-200 dark:border-neutral-800 space-y-2" onClick={(e) => e.stopPropagation()}>
                          <div className="flex items-center justify-between text-[10px] font-medium text-neutral-600 dark:text-neutral-400">
                            <span>Formato:</span>
                            <div className="flex gap-1">
                              {(['csv', 'json', 'markdown', 'html'] as const).map((fmt) => (
                                <button
                                  key={fmt}
                                  onClick={() => setTableExportFormat(fmt)}
                                  className={`px-1.5 py-0.5 rounded text-[9px] uppercase font-mono transition-colors ${
                                    tableExportFormat === fmt
                                      ? 'bg-emerald-600 text-white font-bold'
                                      : 'bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700'
                                  }`}
                                >
                                  {fmt === 'markdown' ? 'MD' : fmt}
                                </button>
                              ))}
                            </div>
                          </div>

                          <div className="grid grid-cols-2 gap-1.5">
                            <button
                              onClick={async () => {
                                if (!onExportTable) return;
                                try {
                                  setIsExportingTable(true);
                                  const content = await onExportTable(table.table_idx, tableExportFormat);
                                  await navigator.clipboard.writeText(content);
                                  setCopiedFormat(tableExportFormat);
                                  setTimeout(() => setCopiedFormat(null), 2000);
                                } catch (err) {
                                  console.error('Failed to copy table export:', err);
                                  alert('Error al exportar tabla.');
                                } finally {
                                  setIsExportingTable(false);
                                }
                              }}
                              disabled={isExportingTable}
                              className="py-1.5 px-2 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-300 rounded text-[10px] font-medium transition-colors flex items-center justify-center gap-1 cursor-pointer"
                            >
                              {copiedFormat === tableExportFormat ? (
                                <>
                                  <Check size={11} className="text-emerald-500" />
                                  <span className="text-emerald-600 dark:text-emerald-400">¡Copiado!</span>
                                </>
                              ) : (
                                <>
                                  <Copy size={11} />
                                  <span>Copiar {tableExportFormat.toUpperCase()}</span>
                                </>
                              )}
                            </button>

                            <button
                              onClick={() => {
                                onDownloadTable?.(table.table_idx, tableExportFormat);
                              }}
                              className="py-1.5 px-2 bg-emerald-600 hover:bg-emerald-700 text-white rounded text-[10px] font-semibold transition-colors flex items-center justify-center gap-1 cursor-pointer shadow-xs"
                            >
                              <Download size={11} />
                              <span>Descargar</span>
                            </button>
                          </div>
                        </div>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}

        {activeTab === 'optimize' && (
          <div className="space-y-4">
            <div>
              <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-1 flex items-center justify-between">
                <span>Motor de Optimización y Compresión</span>
                <span className="text-[9px] text-amber-500 font-mono font-semibold bg-amber-50 dark:bg-amber-950/40 px-1.5 py-0.5 rounded">
                  ISO 32000-1 §7.5.7
                </span>
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400 px-2 leading-relaxed">
                Empaquetado de flujos en <code className="text-amber-600 dark:text-amber-400">/ObjStm</code>, recolección de basura por análisis de alcanzabilidad y deduplicación criptográfica.
              </p>
            </div>

            {/* Opciones de Optimización */}
            <div className="p-3 bg-neutral-50 dark:bg-neutral-800/40 border border-neutral-200 dark:border-neutral-800 rounded-lg space-y-3">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-neutral-400 block mb-1">
                Estrategias de Reducción
              </span>

              {/* Garbage Collection */}
              <label className="flex items-start gap-2.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={optRemoveUnused}
                  onChange={(e) => setOptRemoveUnused(e.target.checked)}
                  className="mt-0.5 rounded text-amber-600 focus:ring-amber-500 border-neutral-300 dark:border-neutral-700"
                />
                <div className="text-xs">
                  <div className="font-medium text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                    <Trash2 size={12} className="text-rose-500" />
                    <span>Garbage Collection (Purga Huérfanos)</span>
                  </div>
                  <div className="text-[10px] text-neutral-500">
                    Elimina objetos COS no alcanzables desde la raíz del catálogo.
                  </div>
                </div>
              </label>

              {/* Object Streams (/ObjStm) */}
              <label className="flex items-start gap-2.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={optPackObjectStreams}
                  onChange={(e) => setOptPackObjectStreams(e.target.checked)}
                  className="mt-0.5 rounded text-amber-600 focus:ring-amber-500 border-neutral-300 dark:border-neutral-700"
                />
                <div className="text-xs">
                  <div className="font-medium text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                    <FileArchive size={12} className="text-amber-500" />
                    <span>Empaquetar en Object Streams</span>
                  </div>
                  <div className="text-[10px] text-neutral-500">
                    Comprime múltiples objetos en contenedores <code className="text-amber-600 dark:text-amber-400">/ObjStm</code> con tabla <code className="text-amber-600 dark:text-amber-400">/XRef</code> moderna.
                  </div>
                </div>
              </label>

              {/* Recompresión Flate */}
              <label className="flex items-start gap-2.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={optRecompressFlate}
                  onChange={(e) => setOptRecompressFlate(e.target.checked)}
                  className="mt-0.5 rounded text-amber-600 focus:ring-amber-500 border-neutral-300 dark:border-neutral-700"
                />
                <div className="text-xs">
                  <div className="font-medium text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                    <Zap size={12} className="text-amber-500" />
                    <span>Recompresión Flate Óptima</span>
                  </div>
                  <div className="text-[10px] text-neutral-500">
                    Recomprime flujos con Zlib Best Compression sin degradar imágenes.
                  </div>
                </div>
              </label>

              {/* Deduplicación Criptográfica */}
              <label className="flex items-start gap-2.5 cursor-pointer">
                <input
                  type="checkbox"
                  checked={optDeduplicateStreams}
                  onChange={(e) => setOptDeduplicateStreams(e.target.checked)}
                  className="mt-0.5 rounded text-amber-600 focus:ring-amber-500 border-neutral-300 dark:border-neutral-700"
                />
                <div className="text-xs">
                  <div className="font-medium text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                    <Layers size={12} className="text-emerald-500" />
                    <span>Deduplicación Criptográfica</span>
                  </div>
                  <div className="text-[10px] text-neutral-500">
                    Detecta streams idénticos vía SHA-256 y unifica referencias COS.
                  </div>
                </div>
              </label>

              {/* Slider de objetos por stream */}
              {optPackObjectStreams && (
                <div className="pt-2 border-t border-neutral-200 dark:border-neutral-700/60">
                  <div className="flex justify-between items-center text-[10px] font-medium text-neutral-600 dark:text-neutral-300 mb-1">
                    <span>Máx. Objetos por Contenedor:</span>
                    <span className="font-mono text-amber-600 font-bold">{optMaxObjectsPerStream}</span>
                  </div>
                  <input
                    type="range"
                    min="20"
                    max="500"
                    step="10"
                    value={optMaxObjectsPerStream}
                    onChange={(e) => setOptMaxObjectsPerStream(parseInt(e.target.value, 10))}
                    className="w-full accent-amber-500 cursor-pointer h-1.5 bg-neutral-200 dark:bg-neutral-700 rounded-lg"
                  />
                </div>
              )}
            </div>

            {/* Error banner */}
            {optimizeError && (
              <div className="p-2.5 bg-rose-50 dark:bg-rose-950/30 border border-rose-200 dark:border-rose-900 rounded-lg text-rose-600 dark:text-rose-400 text-xs">
                {optimizeError}
              </div>
            )}

            {/* Run Button */}
            <button
              onClick={handleRunOptimization}
              disabled={isOptimizing}
              className="w-full py-2 px-3 bg-amber-500 hover:bg-amber-600 disabled:bg-neutral-300 text-white rounded-lg text-xs font-semibold shadow-xs flex items-center justify-center gap-1.5 transition-colors cursor-pointer"
            >
              {isOptimizing ? (
                <>
                  <RefreshCw size={13} className="animate-spin" />
                  <span>Optimizando Flujos y XRefs...</span>
                </>
              ) : (
                <>
                  <Zap size={13} />
                  <span>Ejecutar Optimización</span>
                </>
              )}
            </button>

            {/* Metrics & Report Display */}
            {optimizeStats && (
              <div className="p-3 bg-amber-50/50 dark:bg-amber-950/20 border border-amber-200/80 dark:border-amber-800/60 rounded-xl space-y-3">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-1.5 text-xs font-semibold text-neutral-900 dark:text-neutral-100">
                    <CheckCircle2 size={14} className="text-emerald-500" />
                    <span>Optimización Exitosa</span>
                  </div>
                  <span className="px-2 py-0.5 rounded-full text-[10px] font-bold bg-emerald-500 text-white shadow-xs">
                    -{optimizeStats.compression_ratio_pct.toFixed(1)}%
                  </span>
                </div>

                <div className="grid grid-cols-2 gap-2 text-xs">
                  <div className="p-2 bg-white dark:bg-neutral-800/80 rounded-lg border border-neutral-200/80 dark:border-neutral-700/60">
                    <span className="text-[10px] text-neutral-400 block">Original</span>
                    <span className="font-mono font-semibold text-neutral-700 dark:text-neutral-300">
                      {formatOptBytes(optimizeStats.original_size)}
                    </span>
                  </div>
                  <div className="p-2 bg-white dark:bg-neutral-800/80 rounded-lg border border-emerald-300/80 dark:border-emerald-700/60">
                    <span className="text-[10px] text-emerald-600 dark:text-emerald-400 block font-medium">Optimizado</span>
                    <span className="font-mono font-bold text-emerald-600 dark:text-emerald-400">
                      {formatOptBytes(optimizeStats.optimized_size)}
                    </span>
                  </div>
                </div>

                <div className="text-[10px] space-y-1 text-neutral-600 dark:text-neutral-400 pt-1 border-t border-amber-200/60 dark:border-amber-800/40">
                  <div className="flex justify-between">
                    <span>Ahorro neto:</span>
                    <span className="font-mono font-semibold text-emerald-600 dark:text-emerald-400">
                      {formatOptBytes(optimizeStats.bytes_saved)}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span>Objetos huérfanos purgados:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">
                      {optimizeStats.objects_removed}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span>Flujos recomprimidos:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">
                      {optimizeStats.streams_recompressed}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span>Contenedores /ObjStm creados:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">
                      {optimizeStats.object_streams_created}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span>Flujos deduplicados:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">
                      {optimizeStats.streams_deduplicated}
                    </span>
                  </div>
                </div>

                {/* Actions */}
                <div className="grid grid-cols-2 gap-2 pt-1">
                  <button
                    onClick={handleCopyOptReport}
                    className="py-1.5 px-2 bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-300 rounded text-[10px] font-medium transition-colors flex items-center justify-center gap-1 cursor-pointer"
                  >
                    {copiedOptReport ? (
                      <>
                        <Check size={11} className="text-emerald-500" />
                        <span className="text-emerald-600 dark:text-emerald-400">¡Copiado!</span>
                      </>
                    ) : (
                      <>
                        <Copy size={11} />
                        <span>Copiar Informe</span>
                      </>
                    )}
                  </button>

                  <button
                    type="button"
                    onClick={() => {
                      void downloadAuthorized(
                        getOptimizedExportUrl(documentId),
                        'document_optimized.pdf'
                      );
                    }}
                    className="py-1.5 px-2 bg-amber-500 hover:bg-amber-600 text-white rounded text-[10px] font-semibold transition-colors flex items-center justify-center gap-1 cursor-pointer shadow-xs text-center"
                  >
                    <Download size={11} />
                    <span>Descargar PDF</span>
                  </button>
                </div>
              </div>
            )}
          </div>
        )}

        {activeTab === 'ocr' && (
          <div className="space-y-4">
            <div>
              <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-1">
                Texto buscable
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400 px-2 leading-relaxed">
                Usa tesseract en esta máquina. Un idioma vacío usa eng. La respuesta son conteos: la imagen del escaneo no cambia y el reconocimiento no está garantizado.
              </p>
            </div>

            <div className="p-3 bg-neutral-50 dark:bg-neutral-800/40 border border-neutral-200 dark:border-neutral-800 rounded-lg space-y-2">
              <label className="block text-[10px] font-semibold uppercase tracking-wider text-neutral-400">
                Idioma (traineddata)
                <input
                  type="text"
                  value={ocrLanguage}
                  maxLength={16}
                  onChange={(e) => setOcrLanguage(e.target.value)}
                  placeholder="eng"
                  className="mt-1 w-full rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs font-mono text-neutral-800 dark:text-neutral-100"
                />
              </label>
              <button
                type="button"
                onClick={() => {
                  void handleRecognizeScan();
                }}
                disabled={ocrBusy || pdfaBusy !== null}
                className="w-full py-2 px-3 bg-sky-600 hover:bg-sky-700 disabled:bg-neutral-300 text-white rounded-lg text-xs font-semibold shadow-xs flex items-center justify-center gap-1.5 transition-colors cursor-pointer"
              >
                {ocrBusy ? (
                  <>
                    <RefreshCw size={13} className="animate-spin" />
                    <span>Reconociendo…</span>
                  </>
                ) : (
                  <>
                    <FileCheck size={13} />
                    <span>Reconocer</span>
                  </>
                )}
              </button>
            </div>

            {ocrError && (
              <div className="p-2.5 bg-rose-50 dark:bg-rose-950/30 border border-rose-200 dark:border-rose-900 rounded-lg text-rose-600 dark:text-rose-400 text-xs">
                {ocrError}
              </div>
            )}

            {ocrResult && (
              <div className="p-3 bg-sky-50/60 dark:bg-sky-950/20 border border-sky-200/80 dark:border-sky-800/60 rounded-xl space-y-2">
                <p className="text-[11px] text-neutral-700 dark:text-neutral-200 leading-relaxed">
                  {ocrResult.message}
                </p>
                <div className="text-[10px] space-y-1 text-neutral-600 dark:text-neutral-400">
                  <div className="flex justify-between">
                    <span>Páginas con imagen:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">{ocrResult.pages_seen}</span>
                  </div>
                  <div className="flex justify-between">
                    <span>Páginas reconocidas:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">{ocrResult.pages_recognized}</span>
                  </div>
                  <div className="flex justify-between">
                    <span>Palabras insertadas:</span>
                    <span className="font-mono font-medium text-neutral-800 dark:text-neutral-200">{ocrResult.words_inserted}</span>
                  </div>
                </div>
              </div>
            )}

            <div>
              <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-1">
                PDF/A
              </div>
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400 px-2 leading-relaxed">
                Partes 1b y 2b. Revisar no reescribe el archivo. Convertir sí. La comprobación es estructural: no es veraPDF, ni el preflight de Acrobat, ni una aceptación legal.
              </p>
            </div>

            <div className="p-3 bg-neutral-50 dark:bg-neutral-800/40 border border-neutral-200 dark:border-neutral-800 rounded-lg space-y-2">
              <label className="block text-[10px] font-semibold uppercase tracking-wider text-neutral-400">
                Parte
                <select
                  value={pdfaPart}
                  onChange={(e) => {
                    const next = e.target.value;
                    if (next === '1b' || next === '2b') setPdfaPart(next);
                  }}
                  className="mt-1 w-full rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs text-neutral-800 dark:text-neutral-100"
                >
                  <option value="1b">PDF/A-1b</option>
                  <option value="2b">PDF/A-2b</option>
                </select>
              </label>
              <div className="grid grid-cols-2 gap-2">
                <button
                  type="button"
                  onClick={() => {
                    void handleCheckPdfA();
                  }}
                  disabled={ocrBusy || pdfaBusy !== null}
                  className="py-2 px-2 bg-neutral-800 hover:bg-neutral-900 disabled:bg-neutral-300 text-white rounded-lg text-[10px] font-semibold transition-colors cursor-pointer"
                >
                  {pdfaBusy === 'check' ? 'Revisando…' : 'Revisar'}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    void handleConvertPdfA();
                  }}
                  disabled={ocrBusy || pdfaBusy !== null}
                  className="py-2 px-2 bg-sky-600 hover:bg-sky-700 disabled:bg-neutral-300 text-white rounded-lg text-[10px] font-semibold transition-colors cursor-pointer"
                >
                  {pdfaBusy === 'convert' ? 'Convirtiendo…' : 'Convertir'}
                </button>
              </div>
            </div>

            {pdfaError && (
              <div className="p-2.5 bg-rose-50 dark:bg-rose-950/30 border border-rose-200 dark:border-rose-900 rounded-lg text-rose-600 dark:text-rose-400 text-xs">
                {pdfaError}
              </div>
            )}

            {pdfaResult && (
              <div className="p-3 bg-sky-50/60 dark:bg-sky-950/20 border border-sky-200/80 dark:border-sky-800/60 rounded-xl space-y-2">
                <p className="text-[11px] text-neutral-700 dark:text-neutral-200 leading-relaxed">
                  {pdfaResult.message}
                </p>
                <div className="text-[10px] text-neutral-500">
                  Parte {pdfaResult.part}, conformidad {pdfaResult.conformance}
                </div>
                {pdfaResult.issues.length === 0 ? (
                  <p className="text-[10px] text-neutral-600 dark:text-neutral-400">
                    Sin incidencias en la comprobación estructural.
                  </p>
                ) : (
                  <ul className="max-h-48 overflow-y-auto space-y-1 text-[10px] text-neutral-700 dark:text-neutral-300 list-disc pl-4">
                    {pdfaResult.issues.map((issue, index) => (
                      <li key={`${index}-${issue}`}>{issue}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}
          </div>
        )}

        {activeTab === 'audit' && (
          <div className="space-y-3">
            <div className="flex items-center justify-between px-1">
              <div>
                <div className="text-[11px] font-semibold text-neutral-400 uppercase">
                  Pista de Auditoría ({auditEvents.length})
                </div>
                <p className="text-[10px] text-neutral-500">
                  Eventos inmutables de seguridad y operaciones del documento.
                </p>
              </div>
              <button
                type="button"
                onClick={() => void loadAuditLogs()}
                disabled={isLoadingAudit}
                className="p-1.5 rounded bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-600 dark:text-neutral-300 transition-colors cursor-pointer"
                title="Actualizar eventos"
              >
                <RefreshCw size={12} className={isLoadingAudit ? "animate-spin" : ""} />
              </button>
            </div>

            {auditError && (
              <div className="p-2.5 bg-rose-50 dark:bg-rose-950/30 border border-rose-200 dark:border-rose-900 rounded-lg text-rose-600 dark:text-rose-400 text-xs">
                {auditError}
              </div>
            )}

            {auditEvents.length === 0 && !isLoadingAudit ? (
              <div className="p-4 border border-dashed border-neutral-200 dark:border-neutral-800 rounded-lg text-center text-xs text-neutral-400">
                No hay eventos de auditoría registrados para este tenant.
              </div>
            ) : (
              <div className="space-y-2 max-h-[calc(100vh-14rem)] overflow-y-auto pr-1">
                {auditEvents.map((evt, idx) => {
                  const actionColor =
                    evt.action === 'upload'
                      ? 'bg-blue-100 text-blue-800 dark:bg-blue-900/60 dark:text-blue-300'
                      : evt.action === 'redact'
                      ? 'bg-rose-100 text-rose-800 dark:bg-rose-900/60 dark:text-rose-300'
                      : evt.action === 'sign'
                      ? 'bg-amber-100 text-amber-800 dark:bg-amber-900/60 dark:text-amber-300'
                      : evt.action === 'optimize'
                      ? 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900/60 dark:text-emerald-300'
                      : evt.action === 'metadata_updated'
                      ? 'bg-teal-100 text-teal-800 dark:bg-teal-900/60 dark:text-teal-300'
                      : 'bg-neutral-100 text-neutral-800 dark:bg-neutral-800 dark:text-neutral-300';
                  return (
                    <div
                      key={idx}
                      className="p-2.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 shadow-xs space-y-1"
                    >
                      <div className="flex items-center justify-between">
                        <span className={`text-[10px] font-semibold px-2 py-0.5 rounded uppercase tracking-wider font-mono ${actionColor}`}>
                          {evt.action}
                        </span>
                        <span className="text-[10px] text-neutral-400 font-mono">
                          {new Date(evt.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}
                        </span>
                      </div>
                      <div className="text-[10px] text-neutral-500 font-mono truncate" title={evt.document_id}>
                        Doc: {evt.document_id}
                      </div>
                      <div className="text-[9px] text-neutral-400">
                        {new Date(evt.at).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' })}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {activeTab === 'metadata' && (
          <div className="space-y-3">
            <div className="flex items-center justify-between px-1">
              <div>
                <div className="text-[11px] font-semibold text-neutral-400 uppercase">
                  Metadatos Documentales
                </div>
                <p className="text-[10px] text-neutral-500">
                  Sincronización bidireccional entre /Info y XMP (ISO 32000-1).
                </p>
              </div>
              <button
                type="button"
                onClick={() => void loadMetadata()}
                disabled={loadingMetadata}
                className="p-1.5 rounded bg-neutral-100 hover:bg-neutral-200 dark:bg-neutral-800 dark:hover:bg-neutral-700 text-neutral-600 dark:text-neutral-300 transition-colors cursor-pointer"
                title="Recargar metadatos"
              >
                <RefreshCw size={12} className={loadingMetadata ? "animate-spin" : ""} />
              </button>
            </div>

            {metadataFeedback && (
              <div className="p-2.5 bg-emerald-50 dark:bg-emerald-950/30 border border-emerald-200 dark:border-emerald-900 rounded-lg text-emerald-700 dark:text-emerald-300 text-xs flex items-center gap-1.5">
                <CheckCircle2 size={13} className="shrink-0 text-emerald-600" />
                <span>{metadataFeedback}</span>
              </div>
            )}

            {metadataError && (
              <div className="p-2.5 bg-rose-50 dark:bg-rose-950/30 border border-rose-200 dark:border-rose-900 rounded-lg text-rose-600 dark:text-rose-400 text-xs">
                {metadataError}
              </div>
            )}

            {loadingMetadata ? (
              <div className="p-6 text-center text-xs text-neutral-400 flex flex-col items-center gap-2">
                <RefreshCw size={18} className="animate-spin text-teal-600" />
                <span>Cargando metadatos del documento...</span>
              </div>
            ) : (
              <div className="space-y-3">
                <div className="space-y-1">
                  <label className="text-[10px] font-semibold text-neutral-500 uppercase tracking-wider">
                    Título (/Title & dc:title)
                  </label>
                  <input
                    type="text"
                    value={metadata.title || ''}
                    onChange={(e) => setMetadata({ ...metadata, title: e.target.value })}
                    placeholder="Sin título especificado"
                    className="w-full text-xs px-2.5 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-teal-500"
                  />
                </div>

                <div className="space-y-1">
                  <label className="text-[10px] font-semibold text-neutral-500 uppercase tracking-wider">
                    Autor (/Author & dc:creator)
                  </label>
                  <input
                    type="text"
                    value={metadata.author || ''}
                    onChange={(e) => setMetadata({ ...metadata, author: e.target.value })}
                    placeholder="Autor o entidad emisora"
                    className="w-full text-xs px-2.5 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-teal-500"
                  />
                </div>

                <div className="space-y-1">
                  <label className="text-[10px] font-semibold text-neutral-500 uppercase tracking-wider">
                    Asunto / Descripción (/Subject & dc:description)
                  </label>
                  <textarea
                    rows={2}
                    value={metadata.subject || ''}
                    onChange={(e) => setMetadata({ ...metadata, subject: e.target.value })}
                    placeholder="Tema o descripción del documento"
                    className="w-full text-xs px-2.5 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-teal-500 resize-none"
                  />
                </div>

                <div className="space-y-1">
                  <label className="text-[10px] font-semibold text-neutral-500 uppercase tracking-wider">
                    Palabras Clave (/Keywords & pdf:Keywords)
                  </label>
                  <input
                    type="text"
                    value={metadata.keywords || ''}
                    onChange={(e) => setMetadata({ ...metadata, keywords: e.target.value })}
                    placeholder="palabra1, palabra2, etiquetas"
                    className="w-full text-xs px-2.5 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-teal-500"
                  />
                </div>

                <div className="grid grid-cols-2 gap-2">
                  <div className="space-y-1">
                    <label className="text-[10px] font-semibold text-neutral-500 uppercase tracking-wider">
                      Creador (/Creator)
                    </label>
                    <input
                      type="text"
                      value={metadata.creator || ''}
                      onChange={(e) => setMetadata({ ...metadata, creator: e.target.value })}
                      placeholder="App creadora"
                      className="w-full text-xs px-2 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-teal-500"
                    />
                  </div>
                  <div className="space-y-1">
                    <label className="text-[10px] font-semibold text-neutral-500 uppercase tracking-wider">
                      Productor (/Producer)
                    </label>
                    <input
                      type="text"
                      value={metadata.producer || ''}
                      onChange={(e) => setMetadata({ ...metadata, producer: e.target.value })}
                      placeholder="Motor PDF"
                      className="w-full text-xs px-2 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-teal-500"
                    />
                  </div>
                </div>

                <div className="p-2.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/70 dark:bg-neutral-800/40 space-y-1 text-[11px]">
                  <div className="flex justify-between items-center text-neutral-500">
                    <span>Creación:</span>
                    <span className="font-mono text-[10px] text-neutral-700 dark:text-neutral-300 truncate max-w-[150px]">
                      {metadata.creation_date || 'No registrada'}
                    </span>
                  </div>
                  <div className="flex justify-between items-center text-neutral-500">
                    <span>Modificación:</span>
                    <span className="font-mono text-[10px] text-neutral-700 dark:text-neutral-300 truncate max-w-[150px]">
                      {metadata.mod_date || 'No registrada'}
                    </span>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={handleSaveMetadata}
                  disabled={savingMetadata}
                  className="w-full py-2 px-3 rounded-lg bg-teal-600 hover:bg-teal-700 text-white font-medium text-xs flex items-center justify-center gap-1.5 transition-colors cursor-pointer shadow-xs disabled:opacity-50"
                >
                  {savingMetadata ? (
                    <>
                      <RefreshCw size={13} className="animate-spin" />
                      <span>Sincronizando...</span>
                    </>
                  ) : (
                    <>
                      <FileCheck size={13} />
                      <span>Guardar y Sincronizar Metadatos</span>
                    </>
                  )}
                </button>
              </div>
            )}
          </div>
        )}

        {activeTab === 'diff' && (
          <div className="space-y-4">
            <div className="flex items-center justify-between">
              <span className="text-xs font-semibold text-neutral-500 uppercase tracking-wider">
                Motor de Comparación (Diff)
              </span>
              <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-purple-50 dark:bg-purple-900/30 text-purple-600 dark:text-purple-400">
                LCS Semantic
              </span>
            </div>

            {/* Target Document Selector / Upload */}
            <div className="p-3 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 space-y-2.5">
              <label className="text-[11px] font-medium text-neutral-700 dark:text-neutral-300 block">
                Documento de Comparación (Target Revision)
              </label>

              <div className="space-y-1.5">
                <input
                  type="text"
                  value={diffTargetDocId}
                  onChange={(e) => setDiffTargetDocId(e.target.value)}
                  placeholder="ID de documento target..."
                  className="w-full text-xs px-2.5 py-1.5 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100 focus:outline-hidden focus:ring-1 focus:ring-purple-500 font-mono"
                />

                <div className="flex items-center gap-2">
                  <label className="flex-1 py-1 px-2 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-700 text-neutral-700 dark:text-neutral-300 text-[11px] font-medium flex items-center justify-center gap-1 cursor-pointer transition-colors">
                    <Upload size={12} />
                    <span>Subir PDF para Comparar</span>
                    <input
                      type="file"
                      accept=".pdf"
                      onChange={handleUploadDiffTarget}
                      className="hidden"
                    />
                  </label>
                </div>
              </div>

              {/* Options */}
              <div className="pt-2 border-t border-neutral-200 dark:border-neutral-700/60 space-y-1.5 text-[11px] text-neutral-600 dark:text-neutral-400">
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={diffIgnoreCase}
                    onChange={(e) => setDiffIgnoreCase(e.target.checked)}
                    className="rounded text-purple-600 focus:ring-purple-500"
                  />
                  <span>Ignorar mayúsculas y minúsculas</span>
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={diffCompareImages}
                    onChange={(e) => setDiffCompareImages(e.target.checked)}
                    className="rounded text-purple-600 focus:ring-purple-500"
                  />
                  <span>Comparar imágenes XObject</span>
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={diffCompareMetadata}
                    onChange={(e) => setDiffCompareMetadata(e.target.checked)}
                    className="rounded text-purple-600 focus:ring-purple-500"
                  />
                  <span>Comparar metadatos (/Info y XMP)</span>
                </label>
              </div>

              <button
                type="button"
                onClick={handleRunComparison}
                disabled={isComparing || !diffTargetDocId}
                className="w-full py-2 px-3 rounded-lg bg-purple-600 hover:bg-purple-700 text-white font-medium text-xs flex items-center justify-center gap-1.5 transition-colors cursor-pointer shadow-xs disabled:opacity-50"
              >
                {isComparing ? (
                  <>
                    <RefreshCw size={13} className="animate-spin" />
                    <span>Comparando revisiones...</span>
                  </>
                ) : (
                  <>
                    <GitCompare size={13} />
                    <span>Comparar Revisiones</span>
                  </>
                )}
              </button>
            </div>

            {diffError && (
              <div className="p-2.5 rounded-lg border border-rose-200 dark:border-rose-900/50 bg-rose-50 dark:bg-rose-900/20 text-rose-700 dark:text-rose-300 text-xs flex items-start gap-1.5">
                <AlertCircle size={14} className="shrink-0 mt-0.5" />
                <span>{diffError}</span>
              </div>
            )}

            {/* Results */}
            {diffReport && (
              <div className="space-y-3">
                {/* Status banner */}
                <div
                  className={`p-3 rounded-lg border flex items-center justify-between text-xs font-medium ${
                    diffReport.is_identical
                      ? 'border-emerald-200 dark:border-emerald-800/60 bg-emerald-50 dark:bg-emerald-950/20 text-emerald-700 dark:text-emerald-300'
                      : 'border-purple-200 dark:border-purple-800/60 bg-purple-50 dark:bg-purple-950/20 text-purple-700 dark:text-purple-300'
                  }`}
                >
                  <div className="flex items-center gap-2">
                    {diffReport.is_identical ? (
                      <CheckCircle2 size={16} className="text-emerald-500" />
                    ) : (
                      <GitCompare size={16} className="text-purple-500" />
                    )}
                    <span>
                      {diffReport.is_identical
                        ? 'Documentos Idénticos (Sin diferencias)'
                        : `Diferencias Detectadas (${diffReport.summary.total_pages_with_changes} página(s))`}
                    </span>
                  </div>
                </div>

                {/* Counters grid */}
                <div className="grid grid-cols-3 gap-1.5 text-center">
                  <div className="p-2 rounded border border-emerald-200 dark:border-emerald-900/40 bg-emerald-50/50 dark:bg-emerald-950/20">
                    <div className="text-emerald-600 dark:text-emerald-400 font-bold text-sm">
                      +{diffReport.summary.text_additions + diffReport.summary.image_additions}
                    </div>
                    <div className="text-[9px] uppercase tracking-wider text-emerald-700 dark:text-emerald-300">
                      Adiciones
                    </div>
                  </div>
                  <div className="p-2 rounded border border-rose-200 dark:border-rose-900/40 bg-rose-50/50 dark:bg-rose-950/20">
                    <div className="text-rose-600 dark:text-rose-400 font-bold text-sm">
                      -{diffReport.summary.text_deletions + diffReport.summary.image_deletions}
                    </div>
                    <div className="text-[9px] uppercase tracking-wider text-rose-700 dark:text-rose-300">
                      Eliminaciones
                    </div>
                  </div>
                  <div className="p-2 rounded border border-amber-200 dark:border-amber-900/40 bg-amber-50/50 dark:bg-amber-950/20">
                    <div className="text-amber-600 dark:text-amber-400 font-bold text-sm">
                      ✎{diffReport.summary.text_modifications + diffReport.summary.image_modifications}
                    </div>
                    <div className="text-[9px] uppercase tracking-wider text-amber-700 dark:text-amber-300">
                      Cambios
                    </div>
                  </div>
                </div>

                {/* Metadata discrepancies */}
                {diffReport.metadata_diffs.length > 0 && (
                  <div className="p-2.5 rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/40 space-y-1.5 text-xs">
                    <div className="font-semibold text-neutral-700 dark:text-neutral-300 text-[11px]">
                      Metadatos Modificados ({diffReport.metadata_diffs.length})
                    </div>
                    {diffReport.metadata_diffs.map((md, idx) => (
                      <div key={idx} className="p-1.5 rounded bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 space-y-1 text-[11px]">
                        <div className="font-mono text-purple-600 font-semibold">{md.field}</div>
                        <div className="text-rose-500 line-through text-[10px] truncate">
                          Base: {md.base_value || '(vacío)'}
                        </div>
                        <div className="text-emerald-600 text-[10px] truncate">
                          Target: {md.target_value || '(vacío)'}
                        </div>
                      </div>
                    ))}
                  </div>
                )}

                {/* Per-page differences */}
                <div className="space-y-2">
                  <div className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider">
                    Diferencias por Página
                  </div>
                  {diffReport.pages.map((pDiff, pIdx) => {
                    const pNum = pDiff.page_number_target || pDiff.page_number_base || (pIdx + 1);
                    const totalPageDiffs = pDiff.text_diffs.length + pDiff.image_diffs.length;

                    return (
                      <div
                        key={pIdx}
                        className="rounded-lg border border-neutral-200 dark:border-neutral-800 bg-neutral-50/50 dark:bg-neutral-800/30 overflow-hidden"
                      >
                        <div
                          onClick={() => {
                            onNavigatePage?.(pNum);
                            onSetDiffHighlights?.(pDiff.text_diffs);
                          }}
                          className="p-2 bg-neutral-100/70 dark:bg-neutral-800 flex items-center justify-between text-xs font-medium cursor-pointer hover:bg-neutral-200/50 dark:hover:bg-neutral-700/50 transition-colors"
                        >
                          <div className="flex items-center gap-1.5">
                            <FileText size={12} className="text-neutral-500" />
                            <span>Página {pNum}</span>
                            {pDiff.dimensions_changed && (
                              <span className="text-[9px] px-1 rounded bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300">
                                Dimensiones cambiadas
                              </span>
                            )}
                          </div>
                          <span className="text-[10px] font-mono text-neutral-500">
                            {totalPageDiffs} cambio(s)
                          </span>
                        </div>

                        <div className="p-2 space-y-1.5 text-xs">
                          {pDiff.text_diffs.length === 0 && pDiff.image_diffs.length === 0 ? (
                            <div className="text-[11px] text-neutral-400 italic">Sin cambios en esta página.</div>
                          ) : (
                            pDiff.text_diffs.map((td, tIdx) => (
                              <div
                                key={tIdx}
                                onClick={() => {
                                  onNavigatePage?.(pNum);
                                  onSetDiffHighlights?.([td]);
                                }}
                                className={`p-2 rounded border text-left cursor-pointer transition-all ${
                                  td.kind === 'added'
                                    ? 'border-emerald-200 bg-emerald-50/50 dark:border-emerald-900/40 dark:bg-emerald-950/20'
                                    : td.kind === 'deleted'
                                    ? 'border-rose-200 bg-rose-50/50 dark:border-rose-900/40 dark:bg-rose-950/20'
                                    : 'border-amber-200 bg-amber-50/50 dark:border-amber-900/40 dark:bg-amber-950/20'
                                }`}
                              >
                                <div className="flex items-center justify-between mb-1">
                                  <span
                                    className={`text-[9px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded ${
                                      td.kind === 'added'
                                        ? 'bg-emerald-600 text-white'
                                        : td.kind === 'deleted'
                                        ? 'bg-rose-600 text-white'
                                        : 'bg-amber-600 text-white'
                                    }`}
                                  >
                                    {td.kind === 'added'
                                      ? '+ Añadido'
                                      : td.kind === 'deleted'
                                      ? '- Eliminado'
                                      : '✎ Modificado'}
                                  </span>
                                </div>

                                {td.kind === 'modified' && (
                                  <div className="space-y-1 text-[11px]">
                                    <div className="text-rose-600 dark:text-rose-400 line-through">
                                      {td.base_text}
                                    </div>
                                    <div className="text-emerald-600 dark:text-emerald-400 font-medium">
                                      {td.target_text}
                                    </div>
                                    <div className="flex flex-wrap gap-1 pt-1 border-t border-amber-200 dark:border-amber-800/40">
                                      {td.word_diffs.map((wd, wIdx) => (
                                        <span
                                          key={wIdx}
                                          className={`text-[9px] px-1 rounded ${
                                            wd.kind === 'added'
                                              ? 'bg-emerald-100 dark:bg-emerald-900/40 text-emerald-800 dark:text-emerald-300 font-semibold'
                                              : wd.kind === 'deleted'
                                              ? 'bg-rose-100 dark:bg-rose-900/40 text-rose-800 dark:text-rose-300 line-through'
                                              : 'text-neutral-500'
                                          }`}
                                        >
                                          {wd.text}
                                        </span>
                                      ))}
                                    </div>
                                  </div>
                                )}

                                {td.kind === 'added' && (
                                  <div className="text-emerald-700 dark:text-emerald-300 text-[11px] font-medium">
                                    {td.target_text}
                                  </div>
                                )}

                                {td.kind === 'deleted' && (
                                  <div className="text-rose-700 dark:text-rose-300 text-[11px] line-through">
                                    {td.base_text}
                                  </div>
                                )}
                              </div>
                            ))
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Engine Security & Metrics Footer */}
      <div className="p-4 border-t border-neutral-200 dark:border-neutral-800 bg-neutral-50/80 dark:bg-neutral-800/40 text-[11px] space-y-1.5">
        <div className="flex items-center gap-1.5 font-semibold text-neutral-700 dark:text-neutral-300">
          <ShieldCheck size={13} className="text-emerald-500" />
          <span>ISO 32000 Conformance Active</span>
        </div>
        <div className="text-neutral-500 flex justify-between">
          <span>Decompression Guard:</span>
          <span className="font-mono font-medium text-neutral-700 dark:text-neutral-300">100:1 Bounded</span>
        </div>
        <div className="text-neutral-500 flex justify-between">
          <span>Recursion Depth Cap:</span>
          <span className="font-mono font-medium text-neutral-700 dark:text-neutral-300">64 Levels</span>
        </div>
        <div className="text-neutral-500 flex justify-between">
          <span>Vector Lossless Delta:</span>
          <span className="font-mono font-medium text-emerald-600 dark:text-emerald-400">0.00% Drift</span>
        </div>
      </div>
    </aside>
  );
};
