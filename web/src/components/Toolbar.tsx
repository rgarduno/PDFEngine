'use client';

import React from 'react';
import {
  AlignLeft,
  AlignCenter,
  AlignRight,
  AlignJustify,
  Download,
  Upload,
  Undo2,
  Redo2,
  ZoomIn,
  ZoomOut,
  Sparkles,
  FileCheck2,
  RotateCw,
  Highlighter,
  Underline,
  Link2,
  Award,
} from 'lucide-react';
import { TextAlignment } from '@/lib/types';

interface ToolbarProps {
  filename: string;
  pageNumber: number;
  totalPages: number;
  zoom: number;
  onZoomChange: (newZoom: number) => void;
  selectedAlignment?: TextAlignment;
  onAlignmentChange: (align: TextAlignment) => void;
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onRotateClockwise?: () => void;
  hasSelectedParagraph?: boolean;
  onAddHighlight?: () => void;
  onAddUnderline?: () => void;
  onAddLink?: () => void;
  onAddStamp?: (stampType: string) => void;
  onUploadClick: () => void;
  onExportClick: () => void;
  isExporting: boolean;
  wsConnected: boolean;
}

export const Toolbar: React.FC<ToolbarProps> = ({
  filename,
  pageNumber,
  totalPages,
  zoom,
  onZoomChange,
  selectedAlignment = 'left',
  onAlignmentChange,
  canUndo,
  canRedo,
  onUndo,
  onRedo,
  onRotateClockwise,
  hasSelectedParagraph = false,
  onAddHighlight,
  onAddUnderline,
  onAddLink,
  onAddStamp,
  onUploadClick,
  onExportClick,
  isExporting,
  wsConnected,
}) => {
  return (
    <header className="h-16 border-b border-neutral-200 dark:border-neutral-800 bg-white/95 dark:bg-neutral-900/95 backdrop-blur-md px-4 flex items-center justify-between select-none z-30 sticky top-0">
      {/* Left: Document Info */}
      <div className="flex items-center gap-3 min-w-0">
        <div className="w-9 h-9 rounded-lg bg-red-600 text-white flex items-center justify-center font-bold text-sm shadow-sm">
          PDF
        </div>
        <div className="truncate">
          <div className="flex items-center gap-2">
            <span className="font-semibold text-sm text-neutral-900 dark:text-neutral-100 truncate max-w-[220px]">
              {filename}
            </span>
            <span className="text-[11px] px-2 py-0.5 rounded-full font-medium bg-neutral-100 dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400">
              Page {pageNumber} of {totalPages}
            </span>
          </div>
          <div className="flex items-center gap-1.5 text-[11px] text-neutral-500">
            <span
              className={`inline-block w-2 h-2 rounded-full ${
                wsConnected ? 'bg-emerald-500 animate-pulse' : 'bg-amber-400'
              }`}
            />
            <span>{wsConnected ? 'Surgical Engine: Synchronized' : 'Offline Engine Mode'}</span>
          </div>
        </div>
      </div>

      {/* Center: Formatting & Zoom Controls */}
      <div className="flex items-center gap-1 bg-neutral-100 dark:bg-neutral-800/80 p-1 rounded-lg border border-neutral-200 dark:border-neutral-700/60">
        {/* Undo / Redo */}
        <button
          onClick={onUndo}
          disabled={!canUndo}
          title="Undo (Ctrl+Z / Cmd+Z)"
          className="p-1.5 rounded hover:bg-white dark:hover:bg-neutral-700 disabled:opacity-30 disabled:hover:bg-transparent transition-colors text-neutral-700 dark:text-neutral-300"
        >
          <Undo2 size={16} />
        </button>
        <button
          onClick={onRedo}
          disabled={!canRedo}
          title="Redo (Ctrl+Shift+Z / Cmd+Shift+Z)"
          className="p-1.5 rounded hover:bg-white dark:hover:bg-neutral-700 disabled:opacity-30 disabled:hover:bg-transparent transition-colors text-neutral-700 dark:text-neutral-300"
        >
          <Redo2 size={16} />
        </button>

        <div className="w-px h-5 bg-neutral-300 dark:bg-neutral-700 mx-1" />

        {/* Rotate Page */}
        {onRotateClockwise && (
          <>
            <button
              onClick={onRotateClockwise}
              title="Rotate Page 90° Clockwise"
              className="p-1.5 rounded hover:bg-white dark:hover:bg-neutral-700 transition-colors text-neutral-700 dark:text-neutral-300 cursor-pointer"
            >
              <RotateCw size={16} />
            </button>
            <div className="w-px h-5 bg-neutral-300 dark:bg-neutral-700 mx-1" />
          </>
        )}

        {/* Alignment */}
        <button
          onClick={() => onAlignmentChange('left')}
          title="Align Left"
          className={`p-1.5 rounded transition-colors ${
            selectedAlignment === 'left'
              ? 'bg-white dark:bg-neutral-700 text-blue-600 dark:text-blue-400 shadow-xs'
              : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700/50'
          }`}
        >
          <AlignLeft size={16} />
        </button>
        <button
          onClick={() => onAlignmentChange('center')}
          title="Align Center"
          className={`p-1.5 rounded transition-colors ${
            selectedAlignment === 'center'
              ? 'bg-white dark:bg-neutral-700 text-blue-600 dark:text-blue-400 shadow-xs'
              : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700/50'
          }`}
        >
          <AlignCenter size={16} />
        </button>
        <button
          onClick={() => onAlignmentChange('right')}
          title="Align Right"
          className={`p-1.5 rounded transition-colors ${
            selectedAlignment === 'right'
              ? 'bg-white dark:bg-neutral-700 text-blue-600 dark:text-blue-400 shadow-xs'
              : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700/50'
          }`}
        >
          <AlignRight size={16} />
        </button>
        <button
          onClick={() => onAlignmentChange('justified')}
          title="Justified"
          className={`p-1.5 rounded transition-colors ${
            selectedAlignment === 'justified'
              ? 'bg-white dark:bg-neutral-700 text-blue-600 dark:text-blue-400 shadow-xs'
              : 'text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-700/50'
          }`}
        >
          <AlignJustify size={16} />
        </button>

        {/* Quick Annotations */}
        {hasSelectedParagraph && (
          <>
            <div className="w-px h-5 bg-neutral-300 dark:bg-neutral-700 mx-1" />
            {onAddHighlight && (
              <button
                onClick={onAddHighlight}
                title="Highlight Selected Block"
                className="p-1.5 rounded hover:bg-amber-100 dark:hover:bg-amber-950/40 text-amber-600 dark:text-amber-400 transition-colors cursor-pointer"
              >
                <Highlighter size={16} />
              </button>
            )}
            {onAddUnderline && (
              <button
                onClick={onAddUnderline}
                title="Underline Selected Block"
                className="p-1.5 rounded hover:bg-blue-100 dark:hover:bg-blue-950/40 text-blue-600 dark:text-blue-400 transition-colors cursor-pointer"
              >
                <Underline size={16} />
              </button>
            )}
            {onAddLink && (
              <button
                onClick={onAddLink}
                title="Add Interactive Web Link"
                className="p-1.5 rounded hover:bg-indigo-100 dark:hover:bg-indigo-950/40 text-indigo-600 dark:text-indigo-400 transition-colors cursor-pointer"
              >
                <Link2 size={16} />
              </button>
            )}
          </>
        )}

        {onAddStamp && (
          <>
            <div className="w-px h-5 bg-neutral-300 dark:bg-neutral-700 mx-1" />
            <button
              onClick={() => onAddStamp('APPROVED')}
              title="Add Rubber Stamp: APPROVED"
              className="flex items-center gap-1 px-2 py-1 rounded hover:bg-emerald-100 dark:hover:bg-emerald-950/40 text-emerald-700 dark:text-emerald-400 text-xs font-semibold transition-colors cursor-pointer"
            >
              <Award size={14} />
              <span className="hidden sm:inline">Stamp</span>
            </button>
          </>
        )}

        <div className="w-px h-5 bg-neutral-300 dark:bg-neutral-700 mx-1" />

        {/* Zoom */}
        <button
          onClick={() => onZoomChange(Math.max(0.5, zoom - 0.15))}
          title="Zoom Out"
          className="p-1.5 rounded hover:bg-white dark:hover:bg-neutral-700 transition-colors text-neutral-700 dark:text-neutral-300"
        >
          <ZoomOut size={16} />
        </button>
        <span className="text-xs font-mono font-medium px-1 text-neutral-600 dark:text-neutral-300 min-w-[42px] text-center">
          {Math.round(zoom * 100)}%
        </span>
        <button
          onClick={() => onZoomChange(Math.min(2.0, zoom + 0.15))}
          title="Zoom In"
          className="p-1.5 rounded hover:bg-white dark:hover:bg-neutral-700 transition-colors text-neutral-700 dark:text-neutral-300"
        >
          <ZoomIn size={16} />
        </button>
      </div>

      {/* Right: Actions */}
      <div className="flex items-center gap-2">
        <button
          onClick={onUploadClick}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-neutral-300 dark:border-neutral-700 text-xs font-medium text-neutral-700 dark:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-all cursor-pointer"
        >
          <Upload size={14} />
          <span>Upload PDF</span>
        </button>

        <button
          onClick={onExportClick}
          disabled={isExporting}
          className="flex items-center gap-1.5 px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 active:scale-98 text-white text-xs font-medium transition-all shadow-sm cursor-pointer disabled:opacity-50"
        >
          {isExporting ? (
            <Sparkles size={14} className="animate-spin" />
          ) : (
            <Download size={14} />
          )}
          <span>{isExporting ? 'Compiling PDF...' : 'Export Lossless PDF'}</span>
        </button>
      </div>
    </header>
  );
};
