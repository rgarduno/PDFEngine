'use client';

import React, { useRef, useEffect } from 'react';
import { PageOverviewItem } from '@/lib/types';
import { DOCUMENT_OVERVIEW_WINDOW } from '@/lib/api';
import {
  FileText,
  RotateCw,
  Trash2,
  ArrowUp,
  ArrowDown,
  PanelLeftClose,
  CheckCircle2,
  ChevronRight,
  Maximize2,
} from 'lucide-react';

interface ThumbnailSidebarProps {
  isOpen: boolean;
  onClose: () => void;
  documentId: string;
  currentPage: number;
  totalPages: number;
  pageOverviews: PageOverviewItem[];
  windowOffset: number;
  onShiftWindow?: (offset: number) => void;
  onSelectPage: (pageNumber: number) => void;
  onRotatePage?: (pageNumber: number, degrees: number) => void;
  onDeletePage?: (pageNumber: number) => void;
  onReorderPages?: (newOrder: number[]) => void;
}

export const ThumbnailSidebar: React.FC<ThumbnailSidebarProps> = ({
  isOpen,
  onClose,
  documentId,
  currentPage,
  totalPages,
  pageOverviews,
  windowOffset,
  onShiftWindow,
  onSelectPage,
  onRotatePage,
  onDeletePage,
  onReorderPages,
}) => {
  const activeThumbnailRef = useRef<HTMLDivElement>(null);

  // Auto-scroll active thumbnail into view
  useEffect(() => {
    if (activeThumbnailRef.current) {
      activeThumbnailRef.current.scrollIntoView({
        behavior: 'smooth',
        block: 'nearest',
      });
    }
  }, [currentPage]);

  if (!isOpen) return null;

  // Handle reordering pages (moving page index up or down)
  const handleMovePage = (pageIdx: number, direction: 'up' | 'down', e: React.MouseEvent) => {
    e.stopPropagation();
    if (!onReorderPages) return;

    const currentOrder = pageOverviews.map((p) => p.page_number);
    const targetIdx = direction === 'up' ? pageIdx - 1 : pageIdx + 1;
    if (targetIdx < 0 || targetIdx >= currentOrder.length) return;

    const newOrder = [...currentOrder];
    const temp = newOrder[pageIdx];
    newOrder[pageIdx] = newOrder[targetIdx];
    newOrder[targetIdx] = temp;

    onReorderPages(newOrder);
  };

  return (
    <aside className="w-56 sm:w-64 border-r border-neutral-200 dark:border-neutral-800 bg-neutral-50/90 dark:bg-neutral-900/90 backdrop-blur-md flex flex-col h-[calc(100vh-4rem)] select-none z-20 transition-all duration-200 shadow-sm">
      {/* Sidebar Header */}
      <div className="p-3 border-b border-neutral-200 dark:border-neutral-800 flex items-center justify-between bg-white/60 dark:bg-neutral-800/40">
        <div className="flex items-center gap-1.5 text-xs font-semibold text-neutral-700 dark:text-neutral-200 uppercase tracking-wider">
          <FileText size={14} className="text-blue-500" />
          <span>
            Miniaturas ({pageOverviews.length > 0 ? `${pageOverviews[0].page_number}–${pageOverviews[pageOverviews.length - 1].page_number}` : '0'} / {totalPages})
          </span>
        </div>
        <button
          onClick={onClose}
          title="Cerrar panel de miniaturas (Ctrl+B)"
          className="p-1 rounded hover:bg-neutral-200 dark:hover:bg-neutral-700 text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200 transition-colors cursor-pointer"
        >
          <PanelLeftClose size={15} />
        </button>
      </div>

      {/* Pages Carousel / Thumbnail List */}
      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {pageOverviews.map((page, idx) => {
          const isActive = currentPage === page.page_number;
          const isRotated = page.rotation !== 0;

          return (
            <div
              key={`thumb-${page.page_number}-${idx}`}
              ref={isActive ? activeThumbnailRef : null}
              onClick={() => onSelectPage(page.page_number)}
              className={`group relative p-2.5 rounded-xl border transition-all cursor-pointer flex flex-col items-center gap-2 ${
                isActive
                  ? 'border-blue-500 bg-white dark:bg-neutral-800 ring-2 ring-blue-500/30 shadow-md'
                  : 'border-neutral-200 dark:border-neutral-800 bg-white/80 dark:bg-neutral-850 hover:border-neutral-300 dark:hover:border-neutral-700 hover:shadow-xs'
              }`}
            >
              {/* Header: Page Badge & Status */}
              <div className="w-full flex items-center justify-between text-[11px]">
                <div className="flex items-center gap-1.5">
                  <span
                    className={`font-mono font-bold px-1.5 py-0.5 rounded text-[10px] ${
                      isActive
                        ? 'bg-blue-600 text-white shadow-xs'
                        : 'bg-neutral-100 dark:bg-neutral-700 text-neutral-600 dark:text-neutral-300'
                    }`}
                  >
                    #{page.page_number}
                  </span>
                  {isActive && (
                    <span className="flex items-center gap-1 text-[9px] font-semibold text-emerald-600 dark:text-emerald-400">
                      <span className="w-1.5 h-1.5 rounded-full bg-emerald-500 animate-pulse" />
                      Activa
                    </span>
                  )}
                </div>

                {isRotated && (
                  <span className="text-[9px] font-mono px-1 py-0.2 rounded bg-amber-100 dark:bg-amber-950/60 text-amber-700 dark:text-amber-300">
                    {page.rotation}°
                  </span>
                )}
              </div>

              {/* Proportional Miniature PDF Sheet */}
              <div className="w-full flex items-center justify-center py-1">
                <div
                  style={{
                    transform: `rotate(${page.rotation}deg)`,
                    transformOrigin: 'center center',
                  }}
                  className={`w-28 sm:w-32 aspect-[612/792] bg-white dark:bg-neutral-900 border rounded-xs shadow-xs p-2.5 flex flex-col justify-between overflow-hidden transition-transform duration-200 ${
                    isActive
                      ? 'border-blue-400 dark:border-blue-600 ring-1 ring-blue-300/40'
                      : 'border-neutral-300 dark:border-neutral-700'
                  }`}
                >
                  {/* Simulated Paragraph Layout Lines */}
                  <div className="space-y-1.5 w-full opacity-70">
                    <div className="h-1 bg-neutral-400 dark:bg-neutral-600 rounded-full w-5/6" />
                    <div className="h-1 bg-neutral-300 dark:bg-neutral-700 rounded-full w-full" />
                    <div className="h-1 bg-neutral-300 dark:bg-neutral-700 rounded-full w-4/6" />
                    <div className="h-1 bg-neutral-200 dark:bg-neutral-800 rounded-full w-3/4 mt-2" />
                    <div className="h-1 bg-neutral-300 dark:bg-neutral-700 rounded-full w-full" />
                    <div className="h-1 bg-neutral-200 dark:bg-neutral-800 rounded-full w-2/3" />
                  </div>

                  {/* Snippet / Stats footer in mini sheet */}
                  <div className="text-[7px] text-neutral-400 dark:text-neutral-500 font-mono truncate border-t border-neutral-100 dark:border-neutral-800 pt-1">
                    {page.preview_snippet || `${page.paragraph_count} bloques`}
                  </div>
                </div>
              </div>

              {/* Inline Action Bar on Hover / Active */}
              <div
                onClick={(e) => e.stopPropagation()}
                className="w-full pt-1.5 border-t border-neutral-100 dark:border-neutral-750 flex items-center justify-between text-neutral-500 dark:text-neutral-400"
              >
                <div className="flex items-center gap-0.5">
                  {/* Rotate +90° */}
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      onRotatePage?.(page.page_number, 90);
                    }}
                    title="Rotar +90°"
                    className="p-1 rounded hover:bg-neutral-100 dark:hover:bg-neutral-700 hover:text-blue-600 dark:hover:text-blue-400 transition-colors cursor-pointer"
                  >
                    <RotateCw size={12} />
                  </button>

                  {/* Move Up */}
                  <button
                    onClick={(e) => handleMovePage(idx, 'up', e)}
                    disabled={idx === 0}
                    title="Mover arriba"
                    className="p-1 rounded hover:bg-neutral-100 dark:hover:bg-neutral-700 disabled:opacity-20 hover:text-neutral-800 dark:hover:text-neutral-200 transition-colors cursor-pointer"
                  >
                    <ArrowUp size={12} />
                  </button>

                  {/* Move Down */}
                  <button
                    onClick={(e) => handleMovePage(idx, 'down', e)}
                    disabled={idx === pageOverviews.length - 1}
                    title="Mover abajo"
                    className="p-1 rounded hover:bg-neutral-100 dark:hover:bg-neutral-700 disabled:opacity-20 hover:text-neutral-800 dark:hover:text-neutral-200 transition-colors cursor-pointer"
                  >
                    <ArrowDown size={12} />
                  </button>
                </div>

                {/* Delete Page */}
                {totalPages > 1 && onDeletePage && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      if (confirm(`¿Eliminar la Página ${page.page_number} permanentemente?`)) {
                        onDeletePage(page.page_number);
                      }
                    }}
                    title="Eliminar página"
                    className="p-1 rounded hover:bg-rose-50 dark:hover:bg-rose-950/40 hover:text-rose-600 dark:hover:text-rose-400 transition-colors cursor-pointer"
                  >
                    <Trash2 size={12} />
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {/* Sidebar Footer info */}
      <div className="p-2.5 border-t border-neutral-200 dark:border-neutral-800 bg-white/40 dark:bg-neutral-850/40 text-[10px] text-neutral-400 flex items-center justify-between gap-2">
        <button
          type="button"
          disabled={windowOffset <= 0}
          onClick={() =>
            onShiftWindow?.(Math.max(0, windowOffset - DOCUMENT_OVERVIEW_WINDOW))
          }
          className="px-1.5 py-0.5 rounded border border-neutral-200 dark:border-neutral-700 disabled:opacity-30 cursor-pointer disabled:cursor-default"
        >
          Anterior
        </button>
        <span className="font-mono">{totalPages} Págs</span>
        <button
          type="button"
          disabled={windowOffset + pageOverviews.length >= totalPages}
          onClick={() => onShiftWindow?.(windowOffset + DOCUMENT_OVERVIEW_WINDOW)}
          className="px-1.5 py-0.5 rounded border border-neutral-200 dark:border-neutral-700 disabled:opacity-30 cursor-pointer disabled:cursor-default"
        >
          Siguiente
        </button>
      </div>
    </aside>
  );
};
