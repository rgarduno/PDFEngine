'use client';

import React, { useRef, useState, useEffect } from 'react';
import { FormFieldElement, ImageElement, Paragraph, TextAlignment } from '@/lib/types';
import { getPageFonts, getFontBinaryUrl, getImageBinaryUrl } from '@/lib/api';
import { Check, Edit3, FileText, ImageIcon, Layers, Move, RefreshCw } from 'lucide-react';

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
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const activeTextareaRef = useRef<HTMLTextAreaElement>(null);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editText, setEditText] = useState<string>('');

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
        try {
          const fontUrl = getFontBinaryUrl(documentId, pageNumber, fontName);
          const fontFace = new FontFace(fontName, `url("${fontUrl}")`);
          fontFace
            .load()
            .then((loaded) => {
              if (isMounted) {
                document.fonts.add(loaded);
              }
            })
            .catch((err) => {
              console.debug(`Dynamic font registration note for ${fontName}:`, err);
            });
        } catch (e) {
          console.debug(`FontFace initialization note:`, e);
        }
      });
    });

    return () => {
      isMounted = false;
    };
  }, [documentId, pageNumber]);

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
        setEditingId(null);
      }}
      className="flex-1 overflow-auto bg-neutral-200/70 dark:bg-neutral-950 p-8 flex items-center justify-center min-h-[calc(100vh-4rem)] relative"
    >
      {/* Precision PDF Page Canvas */}
      <div
        style={{
          width: `${PAGE_WIDTH_PTS * zoom}px`,
          height: `${PAGE_HEIGHT_PTS * zoom}px`,
        }}
        className="relative bg-white dark:bg-neutral-900 shadow-2xl rounded-sm transition-all duration-150 border border-neutral-300 dark:border-neutral-800 select-none overflow-hidden"
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
              <img
                src={imageUrl}
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
  );
};
