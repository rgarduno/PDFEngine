'use client';

import React, { useState, useEffect } from 'react';
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
  ChevronLeft,
  ChevronRight,
  Circle,
  Minus,
  PanelLeft,
  PenLine,
  Pentagon,
  Square,
  Zap,
} from 'lucide-react';
import { DrawTool, TextAlignment } from '@/lib/types';

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
  onNavigatePage?: (page: number) => void;
  showThumbnails?: boolean;
  onToggleThumbnails?: () => void;
  onOptimizeClick?: () => void;
  drawTool?: DrawTool | null;
  onDrawToolChange?: (tool: DrawTool | null) => void;
  strokeColor?: string;
  onStrokeColorChange?: (value: string) => void;
  fillColor?: string;
  fillEnabled?: boolean;
  onFillColorChange?: (value: string) => void;
  onFillEnabledChange?: (enabled: boolean) => void;
  shapeLineWidth?: number;
  onShapeLineWidthChange?: (value: number) => void;
  shapeOpacity?: number;
  onShapeOpacityChange?: (value: number) => void;
  drawingEnabled?: boolean;
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
  onNavigatePage,
  showThumbnails = true,
  onToggleThumbnails,
  onOptimizeClick,
  drawTool = null,
  onDrawToolChange,
  strokeColor = '#1d4ed8',
  onStrokeColorChange,
  fillColor = '#fde68a',
  fillEnabled = false,
  onFillColorChange,
  onFillEnabledChange,
  shapeLineWidth = 1.5,
  onShapeLineWidthChange,
  shapeOpacity = 1,
  onShapeOpacityChange,
  drawingEnabled = true,
}) => {
  const [pageInputValue, setPageInputValue] = useState<string>(String(pageNumber));

  useEffect(() => {
    setPageInputValue(String(pageNumber));
  }, [pageNumber]);

  return (
    <header className="border-b border-neutral-200 dark:border-neutral-800 bg-white/95 dark:bg-neutral-900/95 backdrop-blur-md select-none z-30 sticky top-0">
      <div className="h-16 px-4 flex items-center justify-between">
      {/* Left: Document Info & Page Navigator */}
      <div className="flex items-center gap-3 min-w-0">
        {onToggleThumbnails && (
          <button
            onClick={onToggleThumbnails}
            title="Ver miniaturas de páginas (Ctrl+B)"
            className={`p-2 rounded-lg border transition-all flex items-center gap-1.5 text-xs font-semibold cursor-pointer ${
              showThumbnails
                ? 'bg-blue-50 dark:bg-blue-950/60 text-blue-600 dark:text-blue-300 border-blue-300 dark:border-blue-700 shadow-xs'
                : 'bg-white dark:bg-neutral-800 text-neutral-600 dark:text-neutral-400 border-neutral-200 dark:border-neutral-700 hover:bg-neutral-50 dark:hover:bg-neutral-750'
            }`}
          >
            <PanelLeft size={16} />
            <span className="hidden sm:inline">Páginas</span>
            <span className="px-1.5 py-0.2 rounded-full text-[10px] bg-neutral-200 dark:bg-neutral-700 font-mono font-bold">
              {totalPages}
            </span>
          </button>
        )}

        <div className="w-9 h-9 rounded-lg bg-red-600 text-white flex items-center justify-center font-bold text-sm shadow-sm shrink-0">
          PDF
        </div>

        <div className="truncate">
          <div className="flex items-center gap-2">
            <span className="font-semibold text-sm text-neutral-900 dark:text-neutral-100 truncate max-w-[180px] sm:max-w-[240px]">
              {filename}
            </span>

            {/* Interactive Page Navigator */}
            <div className="flex items-center bg-neutral-100 dark:bg-neutral-800 p-0.5 rounded-lg border border-neutral-200 dark:border-neutral-700 text-xs">
              <button
                onClick={() => onNavigatePage?.(Math.max(1, pageNumber - 1))}
                disabled={pageNumber <= 1}
                title="Página Anterior (PageUp / Alt+←)"
                className="p-1 rounded hover:bg-white dark:hover:bg-neutral-700 disabled:opacity-30 disabled:hover:bg-transparent transition-colors text-neutral-700 dark:text-neutral-300 cursor-pointer disabled:cursor-not-allowed"
              >
                <ChevronLeft size={14} />
              </button>

              <div className="flex items-center px-1 font-mono text-neutral-700 dark:text-neutral-300">
                <input
                  type="number"
                  min={1}
                  max={totalPages}
                  value={pageInputValue}
                  onChange={(e) => setPageInputValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      const p = parseInt(pageInputValue, 10);
                      if (!isNaN(p) && p >= 1 && p <= totalPages) {
                        onNavigatePage?.(p);
                      } else {
                        setPageInputValue(String(pageNumber));
                      }
                    }
                  }}
                  onBlur={() => {
                    const p = parseInt(pageInputValue, 10);
                    if (!isNaN(p) && p >= 1 && p <= totalPages) {
                      onNavigatePage?.(p);
                    } else {
                      setPageInputValue(String(pageNumber));
                    }
                  }}
                  className="w-8 text-center bg-white dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-700 rounded px-0.5 py-0.5 text-xs font-semibold focus:outline-hidden focus:ring-1 focus:ring-blue-500"
                />
                <span className="mx-1 text-neutral-400">/</span>
                <span>{totalPages}</span>
              </div>

              <button
                onClick={() => onNavigatePage?.(Math.min(totalPages, pageNumber + 1))}
                disabled={pageNumber >= totalPages}
                title="Página Siguiente (PageDown / Alt+→)"
                className="p-1 rounded hover:bg-white dark:hover:bg-neutral-700 disabled:opacity-30 disabled:hover:bg-transparent transition-colors text-neutral-700 dark:text-neutral-300 cursor-pointer disabled:cursor-not-allowed"
              >
                <ChevronRight size={14} />
              </button>
            </div>
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

        {onOptimizeClick && (
          <button
            onClick={onOptimizeClick}
            title="Optimizar y Comprimir PDF (/ObjStm & Garbage Collection)"
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-amber-300 dark:border-amber-700 bg-amber-50 dark:bg-amber-950/40 text-amber-800 dark:text-amber-300 hover:bg-amber-100 dark:hover:bg-amber-900/50 text-xs font-semibold transition-all cursor-pointer shadow-2xs"
          >
            <Zap size={14} className="text-amber-600 dark:text-amber-400" />
            <span className="hidden md:inline">Optimizar</span>
          </button>
        )}

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
      </div>
      {onDrawToolChange && (
        <DrawStrip
          drawTool={drawTool}
          onDrawToolChange={onDrawToolChange}
          strokeColor={strokeColor}
          onStrokeColorChange={onStrokeColorChange}
          fillColor={fillColor}
          fillEnabled={fillEnabled}
          onFillColorChange={onFillColorChange}
          onFillEnabledChange={onFillEnabledChange}
          shapeLineWidth={shapeLineWidth}
          onShapeLineWidthChange={onShapeLineWidthChange}
          shapeOpacity={shapeOpacity}
          onShapeOpacityChange={onShapeOpacityChange}
          drawingEnabled={drawingEnabled}
        />
      )}
    </header>
  );
};

const DRAW_TOOLS: { id: DrawTool; label: string; icon: React.ReactNode }[] = [
  { id: 'Ink', label: 'Tinta', icon: <PenLine size={14} /> },
  { id: 'Square', label: 'Rectángulo', icon: <Square size={14} /> },
  { id: 'Circle', label: 'Elipse', icon: <Circle size={14} /> },
  { id: 'Line', label: 'Línea', icon: <Minus size={14} /> },
  { id: 'Arrow', label: 'Flecha', icon: <Minus size={14} className="rotate-[-30deg]" /> },
  { id: 'Polygon', label: 'Polígono', icon: <Pentagon size={14} /> },
];

function DrawStrip({
  drawTool,
  onDrawToolChange,
  strokeColor,
  onStrokeColorChange,
  fillColor,
  fillEnabled,
  onFillColorChange,
  onFillEnabledChange,
  shapeLineWidth,
  onShapeLineWidthChange,
  shapeOpacity,
  onShapeOpacityChange,
  drawingEnabled,
}: {
  drawTool: DrawTool | null;
  onDrawToolChange: (tool: DrawTool | null) => void;
  strokeColor: string;
  onStrokeColorChange?: (value: string) => void;
  fillColor: string;
  fillEnabled: boolean;
  onFillColorChange?: (value: string) => void;
  onFillEnabledChange?: (enabled: boolean) => void;
  shapeLineWidth: number;
  onShapeLineWidthChange?: (value: number) => void;
  shapeOpacity: number;
  onShapeOpacityChange?: (value: number) => void;
  drawingEnabled: boolean;
}) {
  return (
    <div className="px-4 py-1.5 border-t border-neutral-200 dark:border-neutral-800 flex flex-wrap items-center gap-2 text-xs">
      {DRAW_TOOLS.map((tool) => {
        const active = drawTool === tool.id;
        return (
          <button
            key={tool.id}
            type="button"
            disabled={!drawingEnabled}
            title={drawingEnabled ? tool.label : 'Gira la página a 0° para dibujar'}
            onClick={() => onDrawToolChange(active ? null : tool.id)}
            className={`flex items-center gap-1 px-2 py-1 rounded-md border cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed ${
              active
                ? 'bg-blue-600 text-white border-blue-600'
                : 'bg-white dark:bg-neutral-800 text-neutral-700 dark:text-neutral-200 border-neutral-300 dark:border-neutral-700'
            }`}
          >
            {tool.icon}
            <span>{tool.label}</span>
          </button>
        );
      })}
      <label className="flex items-center gap-1 text-neutral-600 dark:text-neutral-300">
        Trazo
        <input
          type="color"
          aria-label="Color de trazo"
          value={strokeColor}
          onChange={(event) => onStrokeColorChange?.(event.target.value)}
          className="h-6 w-8 cursor-pointer bg-transparent"
        />
      </label>
      <label className="flex items-center gap-1 text-neutral-600 dark:text-neutral-300">
        <input
          type="checkbox"
          checked={fillEnabled}
          onChange={(event) => onFillEnabledChange?.(event.target.checked)}
        />
        Relleno
        <input
          type="color"
          aria-label="Color de relleno"
          value={fillColor}
          disabled={!fillEnabled}
          onChange={(event) => onFillColorChange?.(event.target.value)}
          className="h-6 w-8 cursor-pointer bg-transparent disabled:opacity-40"
        />
      </label>
      <label className="flex items-center gap-1 text-neutral-600 dark:text-neutral-300">
        Grosor
        <input
          type="number"
          aria-label="Grosor de línea"
          min={0.25}
          max={24}
          step={0.25}
          value={shapeLineWidth}
          onChange={(event) => onShapeLineWidthChange?.(Number(event.target.value))}
          className="w-16 rounded border border-neutral-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-1 py-0.5"
        />
      </label>
      <label className="flex items-center gap-1 text-neutral-600 dark:text-neutral-300">
        Opacidad
        <input
          type="range"
          aria-label="Opacidad"
          min={0.05}
          max={1}
          step={0.05}
          value={shapeOpacity}
          onChange={(event) => onShapeOpacityChange?.(Number(event.target.value))}
        />
        <span className="w-8 font-mono">{Math.round(shapeOpacity * 100)}</span>
      </label>
      {drawTool === 'Polygon' && (
        <span className="text-neutral-500">Doble clic para cerrar el polígono.</span>
      )}
    </div>
  );
}
