'use client';

import React, { useState } from 'react';
import {
  X,
  Zap,
  CheckCircle2,
  Download,
  Layers,
  Trash2,
  FileArchive,
  RefreshCw,
  Copy,
  Check,
  Info,
} from 'lucide-react';
import { OptimizeRequest, OptimizeResponse } from '@/lib/types';
import { optimizeDocument, getOptimizedExportUrl } from '@/lib/api';

interface OptimizeModalProps {
  isOpen: boolean;
  onClose: () => void;
  documentId: string;
  filename: string;
  onOptimizationComplete?: (stats: OptimizeResponse) => void;
}

export const OptimizeModal: React.FC<OptimizeModalProps> = ({
  isOpen,
  onClose,
  documentId,
  filename,
  onOptimizationComplete,
}) => {
  const [removeUnused, setRemoveUnused] = useState<boolean>(true);
  const [packObjectStreams, setPackObjectStreams] = useState<boolean>(true);
  const [recompressFlate, setRecompressFlate] = useState<boolean>(true);
  const [deduplicateStreams, setDeduplicateStreams] = useState<boolean>(true);
  const [maxObjectsPerStream, setMaxObjectsPerStream] = useState<number>(100);

  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<OptimizeResponse | null>(null);
  const [copied, setCopied] = useState<boolean>(false);

  if (!isOpen) return null;

  const handleOptimize = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const payload: OptimizeRequest = {
        remove_unused: removeUnused,
        pack_object_streams: packObjectStreams,
        recompress_flate: recompressFlate,
        deduplicate_streams: deduplicateStreams,
        max_objects_per_stream: maxObjectsPerStream,
      };
      const response = await optimizeDocument(documentId, payload);
      setStats(response);
      onOptimizationComplete?.(response);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Error desconocido al optimizar el documento';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  const formatBytes = (bytes: number): string => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
  };

  const handleCopyReport = () => {
    if (!stats) return;
    const report = [
      `=== REPORTE DE OPTIMIZACIÓN PDF (ISO 32000-1) ===`,
      `Documento: ${filename} (ID: ${documentId})`,
      `Tamaño Original: ${formatBytes(stats.original_size)}`,
      `Tamaño Optimizado: ${formatBytes(stats.optimized_size)}`,
      `Ahorro Total: ${formatBytes(stats.bytes_saved)} (${stats.compression_ratio_pct.toFixed(1)}%)`,
      `Objetos Huérfanos Purgados: ${stats.objects_removed}`,
      `Flujos Recomprimidos: ${stats.streams_recompressed}`,
      `Contenedores /ObjStm Creados: ${stats.object_streams_created}`,
      `Flujos Deduplicados: ${stats.streams_deduplicated}`,
    ].join('\n');
    navigator.clipboard.writeText(report);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-xs p-4 overflow-y-auto">
      <div className="bg-white dark:bg-neutral-900 border border-neutral-200 dark:border-neutral-800 rounded-2xl max-w-xl w-full shadow-2xl overflow-hidden flex flex-col max-h-[90vh]">
        {/* Header */}
        <div className="px-6 py-4 border-b border-neutral-200 dark:border-neutral-800 flex items-center justify-between bg-neutral-50/50 dark:bg-neutral-850/50">
          <div className="flex items-center gap-2.5">
            <div className="p-2 rounded-xl bg-blue-100 dark:bg-blue-900/50 text-blue-600 dark:text-blue-400">
              <Zap size={20} />
            </div>
            <div>
              <h3 className="font-semibold text-neutral-900 dark:text-neutral-100 text-base">
                Optimización y Compresión de PDF
              </h3>
              <p className="text-xs text-neutral-500">
                Flujos de Objetos (/ObjStm ISO 32000-1 §7.5.7) y Garbage Collection
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors cursor-pointer"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content */}
        <div className="p-6 overflow-y-auto space-y-5 flex-1">
          {error && (
            <div className="p-3.5 rounded-xl bg-rose-50 dark:bg-rose-950/40 border border-rose-200 dark:border-rose-900 text-xs text-rose-700 dark:text-rose-300">
              <span className="font-semibold">Error:</span> {error}
            </div>
          )}

          {/* Options Toggles */}
          <div className="space-y-3">
            <h4 className="text-xs font-semibold uppercase tracking-wider text-neutral-500 flex items-center gap-1.5">
              <Layers size={13} />
              <span>Opciones de Optimización</span>
            </h4>

            {/* Garbage Collection */}
            <label className="flex items-start gap-3 p-3 rounded-xl border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-850/50 transition-colors cursor-pointer">
              <input
                type="checkbox"
                checked={removeUnused}
                onChange={(e) => setRemoveUnused(e.target.checked)}
                className="mt-0.5 rounded border-neutral-300 dark:border-neutral-700 text-blue-600 focus:ring-blue-500"
              />
              <div className="text-xs">
                <div className="font-semibold text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                  <Trash2 size={13} className="text-neutral-500" />
                  <span>Eliminar Objetos Huérfanos (Garbage Collection)</span>
                </div>
                <p className="text-neutral-500 mt-0.5">
                  Recorre el grafo de referencias desde la raíz y purga objetos no alcanzables de páginas eliminadas o metadatos viejos.
                </p>
              </div>
            </label>

            {/* Object Streams /ObjStm */}
            <label className="flex items-start gap-3 p-3 rounded-xl border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-850/50 transition-colors cursor-pointer">
              <input
                type="checkbox"
                checked={packObjectStreams}
                onChange={(e) => setPackObjectStreams(e.target.checked)}
                className="mt-0.5 rounded border-neutral-300 dark:border-neutral-700 text-blue-600 focus:ring-blue-500"
              />
              <div className="text-xs">
                <div className="font-semibold text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                  <FileArchive size={13} className="text-neutral-500" />
                  <span>Empaquetar en Flujos de Objetos (/ObjStm - PDF 1.5+)</span>
                </div>
                <p className="text-neutral-500 mt-0.5">
                  Comprime diccionarios, arrays y números dentro de flujos Flate zlib continuos, reduciendo drásticamente el tamaño físico.
                </p>
              </div>
            </label>

            {/* Stream Recompression */}
            <label className="flex items-start gap-3 p-3 rounded-xl border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-850/50 transition-colors cursor-pointer">
              <input
                type="checkbox"
                checked={recompressFlate}
                onChange={(e) => setRecompressFlate(e.target.checked)}
                className="mt-0.5 rounded border-neutral-300 dark:border-neutral-700 text-blue-600 focus:ring-blue-500"
              />
              <div className="text-xs">
                <div className="font-semibold text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                  <RefreshCw size={13} className="text-neutral-500" />
                  <span>Recompresión sin pérdidas Flate (Zlib Best)</span>
                </div>
                <p className="text-neutral-500 mt-0.5">
                  Comprime flujos de texto y vectores al nivel óptimo sin alterar o degradar imágenes binarias (preservación quirúrgica).
                </p>
              </div>
            </label>

            {/* Deduplication */}
            <label className="flex items-start gap-3 p-3 rounded-xl border border-neutral-200 dark:border-neutral-800 hover:bg-neutral-50 dark:hover:bg-neutral-850/50 transition-colors cursor-pointer">
              <input
                type="checkbox"
                checked={deduplicateStreams}
                onChange={(e) => setDeduplicateStreams(e.target.checked)}
                className="mt-0.5 rounded border-neutral-300 dark:border-neutral-700 text-blue-600 focus:ring-blue-500"
              />
              <div className="text-xs">
                <div className="font-semibold text-neutral-800 dark:text-neutral-200 flex items-center gap-1.5">
                  <Layers size={13} className="text-neutral-500" />
                  <span>Deduplicar Recursos y Flujos Idénticos</span>
                </div>
                <p className="text-neutral-500 mt-0.5">
                  Detecta streams idénticos mediante huella criptográfica SHA-256 y unifica sus referencias a una única instancia.
                </p>
              </div>
            </label>

            {/* Max objects slider */}
            {packObjectStreams && (
              <div className="p-3 rounded-xl bg-neutral-50 dark:bg-neutral-800/50 border border-neutral-200 dark:border-neutral-700/60">
                <div className="flex items-center justify-between text-xs mb-1.5">
                  <span className="font-medium text-neutral-700 dark:text-neutral-300">
                    Objetos máximos por contenedor /ObjStm:
                  </span>
                  <span className="font-mono font-semibold text-blue-600 dark:text-blue-400">
                    {maxObjectsPerStream}
                  </span>
                </div>
                <input
                  type="range"
                  min={10}
                  max={200}
                  step={10}
                  value={maxObjectsPerStream}
                  onChange={(e) => setMaxObjectsPerStream(parseInt(e.target.value, 10))}
                  className="w-full accent-blue-600 cursor-pointer"
                />
              </div>
            )}
          </div>

          {/* Results Card */}
          {stats && (
            <div className="p-4 rounded-xl border border-emerald-300 dark:border-emerald-800 bg-emerald-50/60 dark:bg-emerald-950/30 space-y-3">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-emerald-800 dark:text-emerald-300 font-semibold text-xs">
                  <CheckCircle2 size={16} className="text-emerald-600 dark:text-emerald-400" />
                  <span>Resultado de la Optimización</span>
                </div>
                <span className="px-2 py-0.5 rounded-full text-[11px] font-bold bg-emerald-600 text-white">
                  -{stats.compression_ratio_pct.toFixed(1)}%
                </span>
              </div>

              {/* Comparison Stats */}
              <div className="grid grid-cols-2 gap-2 text-xs">
                <div className="bg-white dark:bg-neutral-900 p-2.5 rounded-lg border border-neutral-200 dark:border-neutral-800">
                  <span className="text-[10px] text-neutral-400 uppercase block font-medium">Tamaño Original</span>
                  <span className="text-sm font-semibold font-mono text-neutral-800 dark:text-neutral-200">
                    {formatBytes(stats.original_size)}
                  </span>
                </div>
                <div className="bg-white dark:bg-neutral-900 p-2.5 rounded-lg border border-emerald-300 dark:border-emerald-800">
                  <span className="text-[10px] text-emerald-600 dark:text-emerald-400 uppercase block font-medium">Tamaño Optimizado</span>
                  <span className="text-sm font-bold font-mono text-emerald-600 dark:text-emerald-400">
                    {formatBytes(stats.optimized_size)}
                  </span>
                </div>
              </div>

              {/* Metrics Grid */}
              <div className="grid grid-cols-4 gap-1.5 text-[11px] font-mono text-center">
                <div className="bg-white/80 dark:bg-neutral-900/80 p-1.5 rounded border border-neutral-200 dark:border-neutral-800">
                  <div className="font-bold text-neutral-900 dark:text-neutral-100">{stats.objects_removed}</div>
                  <div className="text-[9px] text-neutral-500">Purgados</div>
                </div>
                <div className="bg-white/80 dark:bg-neutral-900/80 p-1.5 rounded border border-neutral-200 dark:border-neutral-800">
                  <div className="font-bold text-neutral-900 dark:text-neutral-100">{stats.object_streams_created}</div>
                  <div className="text-[9px] text-neutral-500">/ObjStm</div>
                </div>
                <div className="bg-white/80 dark:bg-neutral-900/80 p-1.5 rounded border border-neutral-200 dark:border-neutral-800">
                  <div className="font-bold text-neutral-900 dark:text-neutral-100">{stats.streams_recompressed}</div>
                  <div className="text-[9px] text-neutral-500">Recomprimidos</div>
                </div>
                <div className="bg-white/80 dark:bg-neutral-900/80 p-1.5 rounded border border-neutral-200 dark:border-neutral-800">
                  <div className="font-bold text-neutral-900 dark:text-neutral-100">{stats.streams_deduplicated}</div>
                  <div className="text-[9px] text-neutral-500">Deduplicados</div>
                </div>
              </div>

              <div className="flex items-center justify-between pt-1">
                <button
                  onClick={handleCopyReport}
                  className="flex items-center gap-1 text-[11px] font-medium text-neutral-600 dark:text-neutral-400 hover:text-neutral-900 dark:hover:text-neutral-100 cursor-pointer"
                >
                  {copied ? <Check size={13} className="text-emerald-600" /> : <Copy size={13} />}
                  <span>{copied ? 'Copiado' : 'Copiar Reporte'}</span>
                </button>

                <a
                  href={getOptimizedExportUrl(documentId)}
                  download={`${filename.replace('.pdf', '')}_optimized.pdf`}
                  className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-700 text-white text-xs font-semibold transition-all shadow-xs"
                >
                  <Download size={13} />
                  <span>Descargar Optimizado</span>
                </a>
              </div>
            </div>
          )}

          {!stats && (
            <div className="p-3 rounded-xl bg-blue-50/60 dark:bg-blue-950/30 border border-blue-200 dark:border-blue-900 flex items-start gap-2 text-xs text-blue-700 dark:text-blue-300">
              <Info size={15} className="shrink-0 mt-0.5" />
              <span>
                Esta operación reestructura el archivo PDF compilándolo con tablas /XRef comprimidas y contenedores /ObjStm según el estándar ISO 32000-1 sin degradar la fidelidad tipográfica ni visual.
              </span>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-6 py-3.5 border-t border-neutral-200 dark:border-neutral-800 flex items-center justify-between bg-neutral-50/50 dark:bg-neutral-850/50">
          <button
            onClick={onClose}
            className="px-3.5 py-1.5 rounded-lg text-xs font-medium text-neutral-600 dark:text-neutral-400 hover:bg-neutral-200 dark:hover:bg-neutral-800 transition-colors cursor-pointer"
          >
            {stats ? 'Cerrar' : 'Cancelar'}
          </button>

          <button
            onClick={handleOptimize}
            disabled={isLoading}
            className="flex items-center gap-1.5 px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white text-xs font-semibold transition-all shadow-xs cursor-pointer disabled:opacity-50"
          >
            {isLoading ? (
              <RefreshCw size={14} className="animate-spin" />
            ) : (
              <Zap size={14} />
            )}
            <span>{isLoading ? 'Optimizando...' : stats ? 'Re-optimizar' : 'Ejecutar Optimización'}</span>
          </button>
        </div>
      </div>
    </div>
  );
};
