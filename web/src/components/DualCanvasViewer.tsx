'use client';

import React, { useRef, useState, useEffect } from 'react';
import { AnnotationElement, BoundingBox, DetectedTableItem, DrawTool, FormFieldElement, ImageElement, Paragraph, TextAlignment } from '@/lib/types';
import { fetchAuthorizedBuffer, getPageFonts, getFontBinaryUrl, getImageBinaryUrl } from '@/lib/api';
import { AuthorizedImage } from '@/components/AuthorizedImage';
import { Award, Check, Edit3, ExternalLink, FileText, Highlighter, ImageIcon, Layers, Move, PenTool, RefreshCw, Table, Trash2 } from 'lucide-react';

interface DualCanvasViewerProps {
  paragraphs: Paragraph[];
  selectedParagraphId: number | null;
  onSelectParagraph: (id: number | null) => void;
  onUpdateParagraphText: (id: number, text: string) => void;
  zoom: number;
  activeReflowId: number | null;
  documentId?: string;
  pageNumber?: number;
  images?: ImageElement[];
  selectedImageId?: number | null;
  onSelectImage?: (id: number | null) => void;
  onTriggerReplaceImage?: (id: number) => void;
  forms?: FormFieldElement[];
  selectedFormFieldName?: string | null;
  onSelectFormField?: (name: string | null) => void;
  onUpdateFormFieldValue?: (name: string, value: string) => void;
  rotation?: number;
  annotations?: AnnotationElement[];
  selectedAnnotationId?: number | null;
  onSelectAnnotation?: (id: number | null) => void;
  onDeleteAnnotation?: (id: number) => void;
  onNavigatePage?: (page: number) => void;
  tables?: DetectedTableItem[];
  selectedTableIdx?: number | null;
  onSelectTable?: (idx: number | null) => void;
  drawTool?: DrawTool | null;
  onCommitShape?: (kind: DrawTool, points: number[][]) => void;
}

// Standard US Letter dimensions in PDF Points (72 points/inch)
const PAGE_WIDTH_PTS = 612;
const PAGE_HEIGHT_PTS = 792;

export const DualCanvasViewer: React.FC<DualCanvasViewerProps> = ({
  paragraphs,
  selectedParagraphId,
  onSelectParagraph,
  onUpdateParagraphText,
  zoom,
  activeReflowId,
  documentId,
  pageNumber = 1,
  images = [],
  selectedImageId = null,
  onSelectImage,
  onTriggerReplaceImage,
  forms = [],
  selectedFormFieldName = null,
  onSelectFormField,
  onUpdateFormFieldValue,
  rotation = 0,
  annotations = [],
  selectedAnnotationId = null,
  onSelectAnnotation,
  onDeleteAnnotation,
  onNavigatePage,
  tables = [],
  selectedTableIdx = null,
  onSelectTable,
  drawTool = null,
  onCommitShape,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const pageRef = useRef<HTMLDivElement>(null);
  const dragStart = useRef<number[] | null>(null);
  const activeTextareaRef = useRef<HTMLTextAreaElement>(null);
  const [draft, setDraft] = useState<number[][]>([]);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editText, setEditText] = useState<string>('');

  const isRotated90or270 = rotation === 90 || rotation === 270;
  const containerWidth = (isRotated90or270 ? PAGE_HEIGHT_PTS : PAGE_WIDTH_PTS) * zoom;
  const containerHeight = (isRotated90or270 ? PAGE_WIDTH_PTS : PAGE_HEIGHT_PTS) * zoom;

  // Handle double click or selection to enter edit mode
  const handleParagraphClick = (p: Paragraph, e: React.MouseEvent) => {
    e.stopPropagation();
    onSelectParagraph(p.id);
  };

  const handleParagraphDoubleClick = (p: Paragraph, e: React.MouseEvent) => {
    e.stopPropagation();
    onSelectParagraph(p.id);
    setEditingId(p.id);
    setEditText(p.text);
  };

  useEffect(() => {
    if (editingId !== null && activeTextareaRef.current) {
      activeTextareaRef.current.focus();
      // Move cursor to end of text
      activeTextareaRef.current.selectionStart = activeTextareaRef.current.value.length;
      activeTextareaRef.current.selectionEnd = activeTextareaRef.current.value.length;
    }
  }, [editingId]);

  // Dynamically load and register embedded TrueType/OpenType fonts for this page
  useEffect(() => {
    if (!documentId) return;
    let isMounted = true;

    getPageFonts(documentId, pageNumber).then((data) => {
      if (!isMounted || !data.fonts) return;
      data.fonts.forEach((fontName) => {
        const fontUrl = getFontBinaryUrl(documentId, pageNumber, fontName);
        fetchAuthorizedBuffer(fontUrl)
          .then((buffer) => new FontFace(fontName, buffer).load())
          .then((loaded) => {
            if (isMounted) {
              document.fonts.add(loaded);
            }
          })
          .catch((err) => {
            console.debug(`Dynamic font registration note for ${fontName}:`, err);
          });
      });
    });

    return () => {
      isMounted = false;
    };
  }, [documentId, pageNumber]);

  useEffect(() => {
    setDraft([]);
    dragStart.current = null;
  }, [drawTool, pageNumber, rotation]);

  const clientToPdf = (event: React.PointerEvent): [number, number] | null => {
    if (rotation !== 0 || !pageRef.current) return null;
    const box = pageRef.current.getBoundingClientRect();
    const x = (event.clientX - box.left) / zoom;
    const y = PAGE_HEIGHT_PTS - (event.clientY - box.top) / zoom;
    if (!Number.isFinite(x) || !Number.isFinite(y)) return null;
    return [Math.round(x * 100) / 100, Math.round(y * 100) / 100];
  };

  const onDrawPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!drawTool || rotation !== 0) return;
    const point = clientToPdf(event);
    if (!point) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    if (drawTool === 'Polygon') {
      setDraft((prev) => [...prev, point]);
      return;
    }
    dragStart.current = point;
    setDraft([point]);
  };

  const onDrawPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!drawTool || drawTool === 'Polygon' || !dragStart.current) return;
    const point = clientToPdf(event);
    if (!point) return;
    if (drawTool === 'Ink') {
      setDraft((prev) => {
        const last = prev[prev.length - 1];
        if (last && Math.hypot(point[0] - last[0], point[1] - last[1]) < 0.8) return prev;
        return [...prev, point];
      });
      return;
    }
    setDraft([dragStart.current, point]);
  };

  const onDrawPointerUp = () => {
    if (!drawTool || drawTool === 'Polygon') return;
    setDraft((current) => {
      const dragged = drawTool !== 'Ink';
      const moved = current.length === 2
        && Math.hypot(current[1][0] - current[0][0], current[1][1] - current[0][1]) >= 2;
      const ready = (drawTool === 'Ink' && current.length >= 2) || (dragged && moved);
      if (ready) onCommitShape?.(drawTool, current);
      dragStart.current = null;
      return [];
    });
  };

  const onDrawDoubleClick = (event: React.MouseEvent) => {
    if (drawTool !== 'Polygon') return;
    event.preventDefault();
    event.stopPropagation();
    setDraft((current) => {
      const points = current.slice(0, -1);
      if (points.length >= 3) onCommitShape?.('Polygon', points);
      return [];
    });
  };

  const handleTextChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value;
    setEditText(val);
    if (editingId !== null) {
      onUpdateParagraphText(editingId, val);
    }
  };

  const handleFinishEditing = () => {
    setEditingId(null);
  };

  // Convert PDF coordinate system (origin bottom-left, Y goes UP)
  // to Canvas/Screen coordinate system (origin top-left, Y goes DOWN)
  const pdfToScreenCoordinates = (bbox: Paragraph['bbox']) => {
    const left = bbox.min_x * zoom;
    const top = (PAGE_HEIGHT_PTS - bbox.max_y) * zoom;
    const width = bbox.width * zoom;
    const height = bbox.height * zoom;
    return { left, top, width, height };
  };

  return (
    <div
      ref={containerRef}
      onClick={() => {
        onSelectParagraph(null);
        onSelectImage?.(null);
        onSelectFormField?.(null);
        onSelectAnnotation?.(null);
        setEditingId(null);
      }}
      className="flex-1 overflow-auto bg-neutral-200/70 dark:bg-neutral-950 p-8 flex items-center justify-center min-h-[calc(100vh-4rem)] relative"
    >
      {/* Precision PDF Page Canvas */}
      <div
        style={{
          width: `${containerWidth}px`,
          height: `${containerHeight}px`,
        }}
        className="relative flex items-center justify-center transition-all duration-300"
      >
        <div
          ref={pageRef}
          style={{
            width: `${PAGE_WIDTH_PTS * zoom}px`,
            height: `${PAGE_HEIGHT_PTS * zoom}px`,
            transform: `rotate(${rotation}deg)`,
            transformOrigin: 'center center',
          }}
          className="relative bg-white dark:bg-neutral-900 shadow-2xl rounded-sm transition-all duration-300 border border-neutral-300 dark:border-neutral-800 select-none overflow-hidden"
        >
        {/* Layer 1: High-fidelity Vector Background & Guidelines */}
        <div className="absolute inset-0 pointer-events-none opacity-40">
          {/* Subtle margin guide lines (0.75 in / 54 pt) */}
          <div
            style={{
              top: `${54 * zoom}px`,
              bottom: `${54 * zoom}px`,
              left: `${54 * zoom}px`,
              right: `${54 * zoom}px`,
            }}
            className="absolute border border-dashed border-blue-400/30"
          />
        </div>

        {/* Layer 1.5: Interactive Image XObjects & Surgical Replacement */}
        {images.map((img) => {
          const { left, top, width, height } = pdfToScreenCoordinates(img.bbox);
          const isSelected = selectedImageId === img.id;
          const imageUrl = getImageBinaryUrl(documentId || '', img.id);

          return (
            <div
              key={img.id}
              onClick={(e) => {
                e.stopPropagation();
                onSelectParagraph(null);
                setEditingId(null);
                onSelectImage?.(img.id);
              }}
              style={{
                left: `${left}px`,
                top: `${top}px`,
                width: `${Math.max(width, 32 * zoom)}px`,
                height: `${Math.max(height, 32 * zoom)}px`,
              }}
              className={`absolute transition-all group overflow-hidden border cursor-pointer ${
                isSelected
                  ? 'ring-2 ring-emerald-500 border-emerald-400 z-20 shadow-lg'
                  : 'border-dashed border-amber-500/50 hover:border-amber-500 hover:ring-1 hover:ring-amber-400/80 z-10'
              }`}
            >
              <AuthorizedImage
                url={imageUrl}
                alt={`XObject Image #${img.id}`}
                className="w-full h-full object-fill pointer-events-none select-none bg-neutral-100 dark:bg-neutral-800"
              />

              {/* Status & Replacement Badge */}
              <div
                className={`absolute top-1 left-1 flex items-center gap-1.5 px-1.5 py-0.5 rounded text-[10px] font-mono transition-opacity ${
                  isSelected
                    ? 'bg-emerald-600 text-white opacity-100'
                    : 'bg-black/70 text-amber-300 opacity-0 group-hover:opacity-100'
                }`}
              >
                <ImageIcon size={10} />
                <span>Img #{img.id}</span>
                <span className="text-[9px] opacity-80">({img.width_px}x{img.height_px})</span>
              </div>

              {/* Floating Replace Button */}
              {onTriggerReplaceImage && (
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    onTriggerReplaceImage(img.id);
                  }}
                  title="Replace Image (PNG / JPEG)"
                  className={`absolute bottom-2 right-2 flex items-center gap-1 px-2 py-1 rounded text-xs font-medium shadow-md transition-all ${
                    isSelected
                      ? 'bg-emerald-600 hover:bg-emerald-700 text-white'
                      : 'bg-black/80 hover:bg-black text-white opacity-0 group-hover:opacity-100'
                  }`}
                >
                  <RefreshCw size={12} />
                  <span>Replace</span>
                </button>
              )}
            </div>
          );
        })}

        {/* Layer 1.8: Interactive AcroForm Fields Overlay */}
        {forms
          .filter((f) => f.page_number === pageNumber)
          .map((field) => {
            const { left, top, width, height } = pdfToScreenCoordinates(field.bbox);
            const isSelected = selectedFormFieldName === field.name;

            return (
              <div
                key={field.id}
                onClick={(e) => {
                  e.stopPropagation();
                  onSelectParagraph(null);
                  onSelectImage?.(null);
                  onSelectFormField?.(field.name);
                }}
                style={{
                  left: `${left}px`,
                  top: `${top}px`,
                  width: `${Math.max(width, 24 * zoom)}px`,
                  height: `${Math.max(height, 20 * zoom)}px`,
                }}
                className={`absolute transition-all group z-15 ${
                  isSelected
                    ? 'ring-2 ring-purple-500 shadow-md'
                    : 'hover:ring-1 hover:ring-purple-400'
                }`}
              >
                {/* Form Input based on type */}
                {field.field_type === 'Checkbox' ? (
                  <label className="flex items-center justify-center w-full h-full cursor-pointer bg-white/90 dark:bg-neutral-800/90 border border-purple-400/80 rounded-xs">
                    <input
                      type="checkbox"
                      checked={field.value.toLowerCase() === 'yes' || field.value === '1' || field.value.toLowerCase() === 'true'}
                      onChange={(e) => {
                        onUpdateFormFieldValue?.(field.name, e.target.checked ? 'Yes' : 'Off');
                      }}
                      className="w-3.5 h-3.5 text-purple-600 rounded-xs focus:ring-0 cursor-pointer"
                    />
                  </label>
                ) : field.field_type === 'Choice' ? (
                  <select
                    value={field.value}
                    onChange={(e) => {
                      onUpdateFormFieldValue?.(field.name, e.target.value);
                    }}
                    style={{
                      fontSize: `${11 * zoom}px`,
                    }}
                    className="w-full h-full px-1.5 bg-purple-50/90 dark:bg-purple-950/40 border border-purple-400/80 rounded-xs text-purple-950 dark:text-purple-100 font-sans outline-none focus:border-purple-600"
                  >
                    {field.options.map((opt) => (
                      <option key={opt} value={opt}>
                        {opt}
                      </option>
                    ))}
                  </select>
                ) : field.field_type === 'Signature' ? (
                  <div
                    style={{ fontSize: `${10 * zoom}px` }}
                    className="w-full h-full flex items-center justify-center gap-1.5 bg-purple-50/80 dark:bg-purple-950/50 border-2 border-dashed border-purple-500/80 rounded text-purple-700 dark:text-purple-300 font-medium px-2 select-none"
                  >
                    <PenTool size={Math.max(12 * zoom, 10)} className="text-purple-600 dark:text-purple-400 shrink-0" />
                    <span className="truncate">{field.value || field.alt_name || 'Firma Digital'}</span>
                  </div>
                ) : (
                  <input
                    type="text"
                    value={field.value}
                    placeholder={field.alt_name || field.name}
                    onChange={(e) => {
                      onUpdateFormFieldValue?.(field.name, e.target.value);
                    }}
                    style={{
                      fontSize: `${11 * zoom}px`,
                    }}
                    className="w-full h-full px-2 bg-purple-50/80 dark:bg-purple-950/30 border border-purple-400/80 rounded-xs text-purple-950 dark:text-purple-100 font-sans outline-none focus:border-purple-600 focus:bg-white dark:focus:bg-neutral-900"
                  />
                )}

                {/* Field Badge */}
                <div
                  className={`absolute -top-5 left-0 flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] font-mono transition-opacity ${
                    isSelected
                      ? 'bg-purple-600 text-white opacity-100'
                      : 'bg-black/75 text-purple-300 opacity-0 group-hover:opacity-100'
                  }`}
                >
                  <FileText size={9} />
                  <span>{field.name}</span>
                </div>
              </div>
            );
          })}

        {/* Layer 1.9: Annotations (Highlights, Underlines, Strikeouts, Links, Stamps) */}
        {annotations
          .filter((a) => a.page_number === pageNumber)
          .map((annot) => {
            const { left, top, width, height } = pdfToScreenCoordinates(annot.bbox);
            const isSelected = selectedAnnotationId === annot.id;

            const rgbColor = annot.color
              ? `rgb(${Math.round(annot.color[0] * 255)}, ${Math.round(annot.color[1] * 255)}, ${Math.round(annot.color[2] * 255)})`
              : annot.subtype === 'Highlight'
              ? 'rgb(254, 240, 138)'
              : annot.subtype === 'Underline'
              ? 'rgb(37, 99, 235)'
              : annot.subtype === 'StrikeOut'
              ? 'rgb(220, 38, 38)'
              : annot.subtype === 'Stamp'
              ? 'rgb(16, 185, 129)'
              : 'rgb(59, 130, 246)';

            return (
              <div
                key={annot.id}
                onClick={(e) => {
                  e.stopPropagation();
                  onSelectParagraph(null);
                  onSelectImage?.(null);
                  onSelectFormField?.(null);
                  onSelectAnnotation?.(annot.id);
                }}
                style={{
                  left: `${left}px`,
                  top: `${top}px`,
                  width: `${Math.max(width, 20 * zoom)}px`,
                  height: `${Math.max(height, 14 * zoom)}px`,
                }}
                className={`absolute transition-all cursor-pointer group ${
                  isSelected ? 'ring-2 ring-amber-500 z-25' : 'hover:ring-1 hover:ring-amber-400 z-15'
                }`}
              >
                {/* Visual rendering per subtype */}
                {annot.subtype === 'Highlight' && (
                  <div
                    style={{
                      backgroundColor: annot.color
                        ? `rgba(${Math.round(annot.color[0] * 255)}, ${Math.round(annot.color[1] * 255)}, ${Math.round(annot.color[2] * 255)}, ${annot.opacity || 0.45})`
                        : 'rgba(254, 240, 138, 0.45)',
                      mixBlendMode: 'multiply',
                    }}
                    className="w-full h-full rounded-xs"
                  />
                )}

                {annot.subtype === 'Underline' && (
                  <div className="w-full h-full flex items-end">
                    <div
                      style={{
                        backgroundColor: rgbColor,
                        height: `${Math.max(2 * zoom, 2)}px`,
                      }}
                      className="w-full"
                    />
                  </div>
                )}

                {annot.subtype === 'StrikeOut' && (
                  <div className="w-full h-full flex items-center">
                    <div
                      style={{
                        backgroundColor: rgbColor,
                        height: `${Math.max(1.5 * zoom, 1.5)}px`,
                      }}
                      className="w-full"
                    />
                  </div>
                )}

                {annot.subtype === 'Link' && (
                  <div
                    onClick={(e) => {
                      if (annot.link_type === 'URI' && annot.link_uri) {
                        e.stopPropagation();
                        window.open(annot.link_uri, '_blank', 'noopener,noreferrer');
                      } else if (annot.link_type === 'GoTo' && annot.link_target_page && onNavigatePage) {
                        e.stopPropagation();
                        onNavigatePage(annot.link_target_page);
                      }
                    }}
                    className="w-full h-full border-b-2 border-dashed border-blue-500 bg-blue-400/10 hover:bg-blue-400/25 flex items-center justify-end px-1 cursor-pointer transition-colors"
                    title={annot.link_uri || (annot.link_target_page ? `Jump to Page ${annot.link_target_page}` : 'Link')}
                  >
                    <ExternalLink size={Math.max(10 * zoom, 10)} className="text-blue-600 dark:text-blue-400" />
                  </div>
                )}

                {['Ink', 'Square', 'Circle', 'Line', 'Polygon'].includes(annot.subtype) && (
                  <ShapeMark annot={annot} rgbColor={rgbColor} />
                )}

                {annot.subtype === 'Stamp' && (
                  <div
                    style={{
                      borderColor: rgbColor,
                      color: rgbColor,
                      transform: 'rotate(-4deg)',
                    }}
                    className="w-full h-full border-2 border-dashed rounded flex flex-col items-center justify-center p-1 font-sans font-black tracking-widest bg-white/90 dark:bg-neutral-900/90 shadow-sm uppercase select-none"
                  >
                    <div
                      style={{ fontSize: `${Math.max(11 * zoom, 9)}px` }}
                      className="font-extrabold text-center leading-none"
                    >
                      {annot.stamp_type || 'APPROVED'}
                    </div>
                    {annot.date_str && (
                      <div
                        style={{ fontSize: `${Math.max(7 * zoom, 6)}px` }}
                        className="text-[8px] font-mono opacity-85 mt-0.5 tracking-normal"
                      >
                        {annot.date_str}
                      </div>
                    )}
                  </div>
                )}

                {/* Floating annotation badge on select or hover */}
                <div
                  className={`absolute -top-6 left-0 flex items-center gap-1.5 px-2 py-0.5 rounded text-[10px] font-mono shadow-sm transition-opacity whitespace-nowrap z-30 ${
                    isSelected
                      ? 'bg-amber-600 text-white opacity-100'
                      : 'bg-black/80 text-amber-300 opacity-0 group-hover:opacity-100'
                  }`}
                >
                  <Highlighter size={10} />
                  <span>
                    {annot.subtype} #{annot.id}
                  </span>
                  {annot.contents && (
                    <span className="text-white/80 max-w-[120px] truncate">
                      ({annot.contents})
                    </span>
                  )}
                  {isSelected && onDeleteAnnotation && (
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onDeleteAnnotation(annot.id);
                      }}
                      title="Delete annotation"
                      className="ml-1 hover:text-red-300 text-white/90"
                    >
                      <Trash2 size={10} />
                    </button>
                  )}
                </div>
              </div>
            );
          })}

        {/* Layer 1.95: Extracted Table Bounding Boxes & Grid Badges */}
        {tables.map((table) => {
          const { left, top, width, height } = pdfToScreenCoordinates(table.bbox);
          const isSelected = selectedTableIdx === table.table_idx;

          return (
            <div
              key={`table-${table.table_idx}`}
              onClick={(e) => {
                e.stopPropagation();
                onSelectTable?.(table.table_idx);
              }}
              style={{
                left: `${left}px`,
                top: `${top}px`,
                width: `${width}px`,
                height: `${height}px`,
              }}
              className={`absolute pointer-events-auto cursor-pointer rounded-sm border-2 transition-all z-15 ${
                isSelected
                  ? 'border-emerald-500 bg-emerald-500/10 shadow-lg ring-2 ring-emerald-400/40'
                  : 'border-dashed border-emerald-400/60 bg-emerald-400/5 hover:border-emerald-500 hover:bg-emerald-500/10'
              }`}
            >
              {/* Table identification badge */}
              <div className="absolute -top-6 left-0 flex items-center gap-1 rounded bg-emerald-700 text-white px-2 py-0.5 text-[9px] font-mono shadow-xs select-none">
                <Table size={10} />
                <span>
                  Tabla #{table.table_idx + 1} ({table.row_count}x{table.col_count})
                </span>
              </div>

              {/* Render individual cells with subtle dotted borders */}
              {table.cells.map((cell, cIdx) => {
                const cLeft = (cell.bbox.min_x - table.bbox.min_x) * zoom;
                const cTop = (table.bbox.max_y - cell.bbox.max_y) * zoom;
                const cWidth = cell.bbox.width * zoom;
                const cHeight = cell.bbox.height * zoom;

                return (
                  <div
                    key={`cell-${table.table_idx}-${cIdx}`}
                    style={{
                      left: `${cLeft}px`,
                      top: `${cTop}px`,
                      width: `${cWidth}px`,
                      height: `${cHeight}px`,
                    }}
                    title={`Celda [${cell.row}, ${cell.col}]: ${cell.text}`}
                    className={`absolute border border-emerald-400/20 hover:border-emerald-500/60 hover:bg-emerald-500/20 transition-colors pointer-events-none ${
                      cell.is_header ? 'bg-emerald-600/10 font-semibold' : ''
                    }`}
                  />
                );
              })}
            </div>
          );
        })}

        {drawTool && rotation === 0 && (
          <div
            className="absolute inset-0 z-40"
            style={{ touchAction: 'none', cursor: 'crosshair' }}
            onPointerDown={onDrawPointerDown}
            onPointerMove={onDrawPointerMove}
            onPointerUp={onDrawPointerUp}
            onDoubleClick={onDrawDoubleClick}
            onClick={(event) => event.stopPropagation()}
          >
            <svg viewBox={`0 0 ${PAGE_WIDTH_PTS} ${PAGE_HEIGHT_PTS}`} className="w-full h-full pointer-events-none">
              <g transform={`translate(0 ${PAGE_HEIGHT_PTS}) scale(1 -1)`}>
                <DraftShape kind={drawTool} points={draft} />
              </g>
            </svg>
          </div>
        )}

        {/* Layer 2: Interactive Paragraph Bounding Boxes & Text In-Place Editor */}
        {paragraphs.map((p) => {
          const { left, top, width, height } = pdfToScreenCoordinates(p.bbox);
          const isSelected = selectedParagraphId === p.id;
          const isEditing = editingId === p.id;
          const isReflowing = activeReflowId === p.id;

          const alignClass =
            p.alignment === 'center'
              ? 'text-center'
              : p.alignment === 'right'
              ? 'text-right'
              : p.alignment === 'justified'
              ? 'text-justify'
              : 'text-left';

          return (
            <div
              key={p.id}
              onClick={(e) => handleParagraphClick(p, e)}
              onDoubleClick={(e) => handleParagraphDoubleClick(p, e)}
              style={{
                left: `${left}px`,
                top: `${top}px`,
                width: `${Math.max(width, 120 * zoom)}px`,
                minHeight: `${Math.max(height, 20 * zoom)}px`,
              }}
              className={`absolute transition-all group cursor-text ${
                isSelected
                  ? 'ring-2 ring-blue-500 bg-blue-50/20 dark:bg-blue-900/10 z-20'
                  : 'hover:ring-1 hover:ring-blue-300/80 hover:bg-neutral-50/40 dark:hover:bg-neutral-800/30 z-10'
              }`}
            >
              {/* Badge indicating node ID and live status */}
              {isSelected && (
                <div className="absolute -top-6 left-0 flex items-center gap-1 bg-blue-600 text-white text-[10px] font-mono px-1.5 py-0.5 rounded shadow-sm z-30 select-none">
                  <Edit3 size={10} />
                  <span>Block #{p.id}</span>
                  {isReflowing && (
                    <span className="w-1.5 h-1.5 rounded-full bg-emerald-300 animate-ping" />
                  )}
                  {isEditing && (
                    <button
                      onClick={handleFinishEditing}
                      title="Apply change"
                      className="ml-1 hover:text-emerald-300"
                    >
                      <Check size={10} />
                    </button>
                  )}
                </div>
              )}

              {isEditing ? (
                /* Editable Textarea in place */
                <textarea
                  ref={activeTextareaRef}
                  value={editText}
                  onChange={handleTextChange}
                  onBlur={handleFinishEditing}
                  style={{
                    fontSize: `${(p.fontSize || 12) * zoom}px`,
                    lineHeight: `${(p.leading || 16) * zoom}px`,
                    fontFamily: p.fontFamily ? `"${p.fontFamily}", sans-serif` : 'sans-serif',
                  }}
                  className={`w-full h-full resize-none p-1 bg-white/95 dark:bg-neutral-900/95 text-neutral-900 dark:text-neutral-100 outline-none border-none ${alignClass} focus:ring-0`}
                />
              ) : (
                /* Rendered Text with precise typography */
                <div
                  style={{
                    fontSize: `${(p.fontSize || 12) * zoom}px`,
                    lineHeight: `${(p.leading || 16) * zoom}px`,
                    fontFamily: p.fontFamily ? `"${p.fontFamily}", sans-serif` : 'sans-serif',
                  }}
                  className={`w-full h-full p-1 whitespace-pre-wrap break-words text-neutral-800 dark:text-neutral-200 ${alignClass}`}
                >
                  {p.text}
                </div>
              )}
            </div>
          );
        })}
        </div>
      </div>
    </div>
  );
};

function unitRgb(channels: number[] | undefined, fallback: string): string {
  if (!channels || channels.length < 3) return fallback;
  return `rgb(${channels.slice(0, 3).map((channel) => Math.round(channel * 255)).join(',')})`;
}

function ShapeMark({ annot, rgbColor }: { annot: AnnotationElement; rgbColor: string }) {
  const width = Math.max(annot.bbox.width, 1);
  const height = Math.max(annot.bbox.height, 1);
  const fill = unitRgb(annot.fill_color, 'none');
  const strokeWidth = Math.max(annot.border_width || 1, 0.25);
  const local = (annot.points || []).map(
    (point) => `${point[0] - annot.bbox.min_x},${annot.bbox.max_y - point[1]}`
  );
  const paint = {
    stroke: rgbColor,
    strokeWidth,
    fill: annot.subtype === 'Ink' || annot.subtype === 'Line' ? 'none' : fill,
    opacity: annot.opacity,
  };
  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="w-full h-full overflow-visible" preserveAspectRatio="none">
      {annot.subtype === 'Square' && (
        <rect x={0} y={0} width={width} height={height} {...paint} />
      )}
      {annot.subtype === 'Circle' && (
        <ellipse cx={width / 2} cy={height / 2} rx={width / 2} ry={height / 2} {...paint} />
      )}
      {annot.subtype === 'Polygon' && local.length >= 3 && <polygon points={local.join(' ')} {...paint} />}
      {(annot.subtype === 'Ink' || annot.subtype === 'Line') && local.length >= 2 && (
        <polyline points={local.join(' ')} {...paint} />
      )}
      {annot.subtype === 'Line' && annot.line_ending && local.length >= 2 && (
        <ArrowHead points={annot.points || []} bbox={annot.bbox} color={rgbColor} />
      )}
    </svg>
  );
}

function ArrowHead({
  points,
  bbox,
  color,
}: {
  points: number[][];
  bbox: BoundingBox;
  color: string;
}) {
  const start = points[points.length - 2];
  const end = points[points.length - 1];
  const x1 = start[0] - bbox.min_x;
  const y1 = bbox.max_y - start[1];
  const x2 = end[0] - bbox.min_x;
  const y2 = bbox.max_y - end[1];
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len = Math.max(Math.hypot(dx, dy), 0.001);
  const ux = dx / len;
  const uy = dy / len;
  const size = 8;
  const baseX = x2 - ux * size;
  const baseY = y2 - uy * size;
  const px = -uy;
  const py = ux;
  const left = `${baseX + px * size * 0.45},${baseY + py * size * 0.45}`;
  const right = `${baseX - px * size * 0.45},${baseY - py * size * 0.45}`;
  return <polygon points={`${x2},${y2} ${left} ${right}`} fill={color} />;
}

function DraftShape({ kind, points }: { kind: DrawTool; points: number[][] }) {
  if (points.length === 0) return null;
  const paint = { stroke: '#1d4ed8', strokeWidth: 1.5, fill: 'none' };
  if ((kind === 'Square' || kind === 'Circle' || kind === 'Line' || kind === 'Arrow') && points.length >= 2) {
    const [a, b] = points;
    const x = Math.min(a[0], b[0]);
    const y = Math.min(a[1], b[1]);
    const width = Math.abs(b[0] - a[0]);
    const height = Math.abs(b[1] - a[1]);
    if (kind === 'Square') return <rect x={x} y={y} width={width} height={height} {...paint} />;
    if (kind === 'Circle') {
      return <ellipse cx={x + width / 2} cy={y + height / 2} rx={width / 2} ry={height / 2} {...paint} />;
    }
    return <polyline points={points.map((point) => point.join(',')).join(' ')} {...paint} />;
  }
  return <polyline points={points.map((point) => point.join(',')).join(' ')} {...paint} />;
}
