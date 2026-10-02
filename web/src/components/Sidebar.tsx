'use client';

import React, { useState } from 'react';
import { AnnotationElement, FormFieldElement, ImageElement, Paragraph } from '@/lib/types';
import { getImageBinaryUrl } from '@/lib/api';
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
} from 'lucide-react';
import {
  AddPaginationPayload,
  AddTextWatermarkPayload,
  RedactPatternPayload,
  RedactTextPayload,
  RedactRegionsPayload,
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
}) => {
  const [activeTab, setActiveTab] = useState<'paragraphs' | 'images' | 'forms' | 'annots' | 'pages' | 'watermark' | 'redact'>('paragraphs');
  const [linkInputUrl, setLinkInputUrl] = useState<string>('https://');

  const [pagFormat, setPagFormat] = useState<string>('Página {page} de {total}');
  const [pagPosition, setPagPosition] = useState<'top_left' | 'top_center' | 'top_right' | 'bottom_left' | 'bottom_center' | 'bottom_right'>('bottom_center');
  const [pagFontSize, setPagFontSize] = useState<number>(9);
  const [pagMargin, setPagMargin] = useState<number>(36);
  const [pagSkipFirst, setPagSkipFirst] = useState<boolean>(false);

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
        <div className="mt-3 grid grid-cols-7 gap-0.5 p-0.5 bg-neutral-100 dark:bg-neutral-800 rounded-lg">
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
                      <img
                        src={imageUrl}
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
            <div className="text-[11px] font-semibold text-neutral-400 uppercase px-2 mb-1">
              Interactive Form Fields ({forms.length})
            </div>

            {forms.length === 0 ? (
              <div className="py-8 text-center text-xs text-neutral-400">
                No interactive AcroForm fields in document.
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
                        <span className="font-mono text-xs font-semibold text-purple-600 dark:text-purple-400 flex items-center gap-1 truncate max-w-[150px]">
                          <FileText size={12} />
                          {f.name}
                        </span>
                        <span className="text-[10px] px-1.5 py-0.5 rounded bg-purple-100 dark:bg-purple-950/60 text-purple-700 dark:text-purple-300 font-mono">
                          {f.field_type}
                        </span>
                      </div>

                      {f.alt_name && (
                        <div className="text-[11px] text-neutral-500 mt-1 italic">
                          {f.alt_name}
                        </div>
                      )}

                      {/* Field Value Input directly in Sidebar */}
                      <div className="mt-2" onClick={(e) => e.stopPropagation()}>
                        {f.field_type === 'Checkbox' ? (
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
                        <span>Page {f.page_number}</span>
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
                <span>ISO 32000-1 §14.11 Legal Redaction</span>
              </div>
              <p className="text-[11px] leading-relaxed text-rose-800 dark:text-rose-300/90">
                Los glifos y streams son destruidos e invalidados físicamente del AST binario en lugar de ocultarse con una capa visual. Operación irreversible.
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
