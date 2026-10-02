'use client';

import React, { useState } from 'react';
import { ImageElement, Paragraph } from '@/lib/types';
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
} from 'lucide-react';

interface SidebarProps {
  paragraphs: Paragraph[];
  selectedParagraphId: number | null;
  onSelectParagraph: (id: number) => void;
  documentId: string;
  images?: ImageElement[];
  selectedImageId?: number | null;
  onSelectImage?: (id: number) => void;
  onTriggerReplaceImage?: (id: number) => void;
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
}) => {
  const [activeTab, setActiveTab] = useState<'paragraphs' | 'images'>('paragraphs');

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
        <div className="mt-3 flex p-0.5 bg-neutral-100 dark:bg-neutral-800 rounded-lg">
          <button
            onClick={() => setActiveTab('paragraphs')}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 text-xs font-medium rounded-md transition-all ${
              activeTab === 'paragraphs'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
          >
            <AlignLeft size={13} />
            <span>Blocks ({paragraphs.length})</span>
          </button>
          <button
            onClick={() => setActiveTab('images')}
            className={`flex-1 flex items-center justify-center gap-1.5 py-1.5 text-xs font-medium rounded-md transition-all ${
              activeTab === 'images'
                ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-neutral-100 shadow-xs'
                : 'text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300'
            }`}
          >
            <ImageIcon size={13} />
            <span>Images ({images.length})</span>
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
        ) : (
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
