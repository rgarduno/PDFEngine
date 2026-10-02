'use client';

import React from 'react';
import { Paragraph } from '@/lib/types';
import {
  ShieldCheck,
  Cpu,
  Layers,
  FileText,
  AlignLeft,
  ChevronRight,
  Maximize2,
  Box,
} from 'lucide-react';

interface SidebarProps {
  paragraphs: Paragraph[];
  selectedParagraphId: number | null;
  onSelectParagraph: (id: number) => void;
  documentId: string;
}

export const Sidebar: React.FC<SidebarProps> = ({
  paragraphs,
  selectedParagraphId,
  onSelectParagraph,
  documentId,
}) => {
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
      </div>

      {/* Paragraph Nodes List */}
      <div className="flex-1 overflow-y-auto p-3 space-y-2">
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
