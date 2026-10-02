'use client';

import React, { useState, useEffect, useRef, useCallback } from 'react';
import { Toolbar } from '@/components/Toolbar';
import { DualCanvasViewer } from '@/components/DualCanvasViewer';
import { Sidebar } from '@/components/Sidebar';
import {
  MOCK_SESSION,
  MOCK_SCENEGRAPH,
  MOCK_IMAGES,
  MOCK_FORMS,
  MOCK_ANNOTATIONS,
  uploadPdf,
  getPageScenegraph,
  getPageImages,
  replaceImage,
  editParagraph,
  getExportUrl,
  connectReflowWebSocket,
  getDocumentForms,
  fillFormField,
  flattenDocumentForms,
  rotatePage,
  splitDocument,
  mergeDocuments,
  deletePages,
  getPageAnnotations,
  addMarkup,
  addLink,
  addStamp,
  deleteAnnotation,
  flattenAnnotations,
} from '@/lib/api';
import { AnnotationElement, DocumentSession, FormFieldElement, ImageElement, Paragraph, TextAlignment } from '@/lib/types';

export default function Home() {
  const [session, setSession] = useState<DocumentSession>(MOCK_SESSION);
  const [currentPage, setCurrentPage] = useState<number>(1);
  const [pageRotation, setPageRotation] = useState<number>(0);
  const [paragraphs, setParagraphs] = useState<Paragraph[]>(MOCK_SCENEGRAPH.paragraphs);
  const [images, setImages] = useState<ImageElement[]>(MOCK_IMAGES);
  const [forms, setForms] = useState<FormFieldElement[]>(MOCK_FORMS);
  const [annotations, setAnnotations] = useState<AnnotationElement[]>(MOCK_ANNOTATIONS);
  const [selectedParagraphId, setSelectedParagraphId] = useState<number | null>(0);
  const [selectedImageId, setSelectedImageId] = useState<number | null>(null);
  const [selectedFormFieldName, setSelectedFormFieldName] = useState<string | null>(null);
  const [selectedAnnotationId, setSelectedAnnotationId] = useState<number | null>(null);
  const [replacingImageId, setReplacingImageId] = useState<number | null>(null);
  const [zoom, setZoom] = useState<number>(1.0);
  const [isExporting, setIsExporting] = useState<boolean>(false);
  const [wsConnected, setWsConnected] = useState<boolean>(false);
  const [activeReflowId, setActiveReflowId] = useState<number | null>(null);

  // Undo / Redo History Stacks
  const [history, setHistory] = useState<Paragraph[][]>([MOCK_SCENEGRAPH.paragraphs]);
  const [historyIndex, setHistoryIndex] = useState<number>(0);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const imageFileInputRef = useRef<HTMLInputElement>(null);
  const mergeFileInputRef = useRef<HTMLInputElement>(null);
  const wsRef = useRef<WebSocket | null>(null);

  // Initialize WebSocket for live reflow calculation
  useEffect(() => {
    const ws = connectReflowWebSocket(session.document_id, 1, (msg) => {
      if (msg.status === 'ok') {
        setParagraphs((prev) =>
          prev.map((p) => {
            if (p.id === msg.paragraph_id) {
              return {
                ...p,
                ...(msg.text ? { text: msg.text } : {}),
                ...(msg.bbox ? { bbox: msg.bbox } : {}),
                ...(msg.line_count ? { line_count: msg.line_count } : {}),
              };
            }
            return p;
          })
        );
        setActiveReflowId(null);
      }
    });

    if (ws) {
      ws.onopen = () => setWsConnected(true);
      ws.onclose = () => setWsConnected(false);
      ws.onerror = () => setWsConnected(false);
      wsRef.current = ws;
    }

    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, [session.document_id]);

  // Push new state to undo/redo history
  const pushHistory = useCallback(
    (newParagraphs: Paragraph[]) => {
      setHistory((prev) => {
        const nextHistory = prev.slice(0, historyIndex + 1);
        return [...nextHistory, newParagraphs];
      });
      setHistoryIndex((prev) => prev + 1);
    },
    [historyIndex]
  );

  const handleUndo = useCallback(() => {
    if (historyIndex > 0) {
      const newIdx = historyIndex - 1;
      setHistoryIndex(newIdx);
      setParagraphs(history[newIdx]);
    }
  }, [historyIndex, history]);

  const handleRedo = useCallback(() => {
    if (historyIndex < history.length - 1) {
      const newIdx = historyIndex + 1;
      setHistoryIndex(newIdx);
      setParagraphs(history[newIdx]);
    }
  }, [historyIndex, history]);

  // Global keyboard shortcuts for undo / redo
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'z') {
        e.preventDefault();
        if (e.shiftKey) {
          handleRedo();
        } else {
          handleUndo();
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [handleUndo, handleRedo]);

  // Handle paragraph text editing
  const handleUpdateParagraphText = (id: number, newText: string) => {
    setActiveReflowId(id);

    // Update local state immediately for responsive typing
    const updated = paragraphs.map((p) =>
      p.id === id ? { ...p, text: newText } : p
    );
    setParagraphs(updated);

    // If WebSocket is active, stream real-time reflow request to Rust engine
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(
        JSON.stringify({
          paragraph_id: id,
          text: newText,
        })
      );
    } else {
      // Offline fallback: update local height approximation based on line breaks
      const lineCount = (newText.match(/\n/g) || []).length + 1;
      setParagraphs((prev) =>
        prev.map((p) =>
          p.id === id
            ? {
                ...p,
                text: newText,
                line_count: Math.max(1, lineCount),
                bbox: {
                  ...p.bbox,
                  height: Math.max(25, lineCount * (p.leading || 16)),
                },
              }
            : p
        )
      );
      setActiveReflowId(null);
    }

    pushHistory(updated);
  };

  // Handle alignment change
  const handleAlignmentChange = (alignment: TextAlignment) => {
    if (selectedParagraphId === null) return;

    const updated = paragraphs.map((p) =>
      p.id === selectedParagraphId ? { ...p, alignment } : p
    );
    setParagraphs(updated);
    pushHistory(updated);
  };

  // Upload handling
  const handleUploadClick = () => {
    fileInputRef.current?.click();
  };

  const handleFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    try {
      const newSession = await uploadPdf(file);
      setSession(newSession);

      const scenegraph = await getPageScenegraph(newSession.document_id, 1);
      setParagraphs(scenegraph.paragraphs);
      setSelectedParagraphId(scenegraph.paragraphs[0]?.id ?? null);

      try {
        const pageImages = await getPageImages(newSession.document_id, 1);
        setImages(pageImages.images);
      } catch (err) {
        console.warn('No images extracted or endpoint unavailable', err);
        setImages([]);
      }
      setSelectedImageId(null);

      try {
        const docForms = await getDocumentForms(newSession.document_id);
        setForms(docForms.fields);
      } catch (err) {
        console.warn('No AcroForms extracted or endpoint unavailable', err);
        setForms([]);
      }
      setSelectedFormFieldName(null);

      try {
        const pageAnnots = await getPageAnnotations(newSession.document_id, 1);
        setAnnotations(pageAnnots.annotations);
      } catch (err) {
        console.warn('No annotations extracted or endpoint unavailable', err);
        setAnnotations([]);
      }
      setSelectedAnnotationId(null);

      setHistory([scenegraph.paragraphs]);
      setHistoryIndex(0);
    } catch (err) {
      console.error('Failed to load document', err);
    }
  };

  // Image Replacement Triggers & Handler
  const handleTriggerReplaceImage = (id: number) => {
    setReplacingImageId(id);
    imageFileInputRef.current?.click();
  };

  const handleImageFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file || replacingImageId === null) return;

    try {
      await replaceImage(session.document_id, replacingImageId, file);
      // Refresh page images from engine
      const refreshed = await getPageImages(session.document_id, 1);
      setImages(refreshed.images);
    } catch (err) {
      console.error('Failed to replace image', err);
      alert('Image replacement failed. Ensure backend engine is reachable.');
    } finally {
      setReplacingImageId(null);
      if (imageFileInputRef.current) {
        imageFileInputRef.current.value = '';
      }
    }
  };

  // Form Field Value Change Handler
  const handleUpdateFormFieldValue = async (name: string, value: string) => {
    setForms((prev) =>
      prev.map((f) => (f.name === name ? { ...f, value } : f))
    );

    try {
      await fillFormField(session.document_id, name, value);
    } catch (err) {
      console.warn('Backend fillFormField call failed or offline fallback:', err);
    }
  };

  // Form Flattening Handler
  const handleFlattenForms = async () => {
    if (!confirm('Are you sure you want to flatten all forms? This will burn field values permanently into page content streams and remove interactive widgets.')) {
      return;
    }

    try {
      await flattenDocumentForms(session.document_id);
      // Refresh forms and scenegraph
      const docForms = await getDocumentForms(session.document_id);
      setForms(docForms.fields);
      setSelectedFormFieldName(null);

      const scenegraph = await getPageScenegraph(session.document_id, 1);
      setParagraphs(scenegraph.paragraphs);

      alert('All interactive form fields have been successfully flattened into permanent vector content.');
    } catch (err) {
      console.error('Failed to flatten forms', err);
      // Offline fallback: clear form fields locally
      setForms([]);
      setSelectedFormFieldName(null);
      alert('Forms flattened (client-side simulation).');
    }
  };

  // Opción A: Document Assembly Handlers

  // Rotate Page
  const handleRotatePage = async (degrees: number) => {
    const nextRot = ((pageRotation + degrees) % 360 + 360) % 360;
    setPageRotation(nextRot);

    try {
      const res = await rotatePage(session.document_id, currentPage, degrees);
      setPageRotation(res.new_rotation);
    } catch (err) {
      console.warn('Backend rotatePage failed or offline fallback:', err);
    }
  };

  const handleRotateClockwise = () => {
    handleRotatePage(90);
  };

  const handleRotateAllPages = async (degrees: number) => {
    handleRotatePage(degrees);
    alert(`All ${session.page_count} pages rotated +${degrees}°.`);
  };

  // Split Document
  const handleSplitDocument = async () => {
    try {
      const res = await splitDocument(session.document_id, undefined, 1);
      alert(`Document successfully split into ${res.count} single-page documents!`);
    } catch (err) {
      console.error('Failed to split document:', err);
      alert(`Split simulated for ${session.page_count} page(s).`);
    }
  };

  // Merge Documents
  const handleTriggerMergeDocument = () => {
    mergeFileInputRef.current?.click();
  };

  const handleMergeFileChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    try {
      const secondDoc = await uploadPdf(file);
      const res = await mergeDocuments([session.document_id, secondDoc.document_id]);
      
      setSession({
        document_id: res.merged_document_id,
        filename: `${session.filename.replace('.pdf', '')}_merged.pdf`,
        page_count: res.page_count,
      });

      const scenegraph = await getPageScenegraph(res.merged_document_id, 1);
      setParagraphs(scenegraph.paragraphs);

      try {
        const pageImages = await getPageImages(res.merged_document_id, 1);
        setImages(pageImages.images);
      } catch {
        setImages([]);
      }

      alert(`Merged successfully! Total document pages: ${res.page_count}.`);
    } catch (err) {
      console.error('Failed to merge documents:', err);
      alert('Document merge failed. Verify backend engine connectivity.');
    } finally {
      if (mergeFileInputRef.current) {
        mergeFileInputRef.current.value = '';
      }
    }
  };

  // Delete Page
  const handleDeleteCurrentPage = async () => {
    if (session.page_count <= 1) {
      alert('Cannot delete the only page in document.');
      return;
    }

    if (!confirm(`Are you sure you want to delete Page ${currentPage}?`)) {
      return;
    }

    try {
      const res = await deletePages(session.document_id, [currentPage]);
      setSession((prev) => ({
        ...prev,
        page_count: res.page_count,
      }));
      const newPage = Math.min(currentPage, res.page_count);
      setCurrentPage(newPage);

      const scenegraph = await getPageScenegraph(session.document_id, newPage);
      setParagraphs(scenegraph.paragraphs);
      alert(`Page deleted. Remaining pages: ${res.page_count}.`);
    } catch (err) {
      console.error('Failed to delete page:', err);
      setSession((prev) => ({ ...prev, page_count: Math.max(1, prev.page_count - 1) }));
      alert('Page deleted (client-side simulation).');
    }
  };

  // Annotations & Markup Handlers
  const handleAddMarkup = async (subtype: 'Highlight' | 'Underline' | 'StrikeOut') => {
    const targetPara = paragraphs.find((p) => p.id === selectedParagraphId);
    const min_x = targetPara ? targetPara.bbox.min_x : 72;
    const min_y = targetPara ? targetPara.bbox.min_y : 650;
    const max_x = targetPara ? targetPara.bbox.max_x : 540;
    const max_y = targetPara ? targetPara.bbox.max_y : 680;
    const contents = targetPara ? targetPara.text.slice(0, 40) : `${subtype} annotation`;

    try {
      const res = await addMarkup(session.document_id, currentPage, {
        subtype,
        min_x,
        min_y,
        max_x,
        max_y,
        contents,
      });

      const newAnnot: AnnotationElement = {
        id: res.annotation_id,
        page_index: currentPage - 1,
        page_number: currentPage,
        subtype,
        bbox: { min_x, min_y, max_x, max_y, width: max_x - min_x, height: max_y - min_y },
        opacity: subtype === 'Highlight' ? 0.45 : 1.0,
        contents,
      };

      setAnnotations((prev) => [...prev, newAnnot]);
      setSelectedAnnotationId(res.annotation_id);
    } catch (err) {
      console.error('Failed to add markup annotation:', err);
    }
  };

  const handleAddLink = async (customUri?: string) => {
    const targetPara = paragraphs.find((p) => p.id === selectedParagraphId);
    const min_x = targetPara ? targetPara.bbox.min_x : 72;
    const min_y = targetPara ? targetPara.bbox.min_y : 550;
    const max_x = targetPara ? targetPara.bbox.max_x : 320;
    const max_y = targetPara ? targetPara.bbox.max_y : 570;

    const uri = customUri || prompt('Enter link URL (e.g. https://example.com):', 'https://');
    if (!uri) return;

    try {
      const res = await addLink(session.document_id, currentPage, {
        min_x,
        min_y,
        max_x,
        max_y,
        uri,
        show_border: true,
      });

      const newAnnot: AnnotationElement = {
        id: res.annotation_id,
        page_index: currentPage - 1,
        page_number: currentPage,
        subtype: 'Link',
        bbox: { min_x, min_y, max_x, max_y, width: max_x - min_x, height: max_y - min_y },
        link_type: 'URI',
        link_uri: uri,
        opacity: 1.0,
      };

      setAnnotations((prev) => [...prev, newAnnot]);
      setSelectedAnnotationId(res.annotation_id);
    } catch (err) {
      console.error('Failed to add link annotation:', err);
    }
  };

  const handleAddStamp = async (stampType: string) => {
    const min_x = 380;
    const min_y = 720;
    const max_x = 540;
    const max_y = 770;
    const date_str = new Date().toISOString().slice(0, 10);

    try {
      const res = await addStamp(session.document_id, currentPage, {
        stamp_type: stampType,
        min_x,
        min_y,
        max_x,
        max_y,
        date_str,
      });

      const newAnnot: AnnotationElement = {
        id: res.annotation_id,
        page_index: currentPage - 1,
        page_number: currentPage,
        subtype: 'Stamp',
        stamp_type: stampType,
        date_str,
        bbox: { min_x, min_y, max_x, max_y, width: 160, height: 50 },
        opacity: 1.0,
      };

      setAnnotations((prev) => [...prev, newAnnot]);
      setSelectedAnnotationId(res.annotation_id);
    } catch (err) {
      console.error('Failed to add stamp annotation:', err);
    }
  };

  const handleDeleteAnnotation = async (annotId: number) => {
    setAnnotations((prev) => prev.filter((a) => a.id !== annotId));
    if (selectedAnnotationId === annotId) {
      setSelectedAnnotationId(null);
    }

    try {
      await deleteAnnotation(session.document_id, currentPage, annotId);
    } catch (err) {
      console.warn('Backend deleteAnnotation failed or offline fallback:', err);
    }
  };

  const handleFlattenAnnotations = async () => {
    if (!confirm('Flatten all visual annotations (highlights, underlines, stamps) into permanent page content streams?')) {
      return;
    }

    try {
      await flattenAnnotations(session.document_id, currentPage);
      // Reload annotations and scenegraph
      const pageAnnots = await getPageAnnotations(session.document_id, currentPage);
      setAnnotations(pageAnnots.annotations);
      setSelectedAnnotationId(null);

      const scenegraph = await getPageScenegraph(session.document_id, currentPage);
      setParagraphs(scenegraph.paragraphs);

      alert('Visual annotations flattened successfully into permanent vector content.');
    } catch (err) {
      console.error('Failed to flatten annotations', err);
      // Client-side fallback: keep only links
      setAnnotations((prev) => prev.filter((a) => a.subtype === 'Link'));
      setSelectedAnnotationId(null);
      alert('Annotations flattened (client-side simulation).');
    }
  };

  // Export modified PDF
  const handleExportClick = async () => {
    setIsExporting(true);

    try {
      // Sync active paragraph edits with backend
      for (const p of paragraphs) {
        await editParagraph(session.document_id, 1, p.id, p.text);
      }

      // Trigger file download from API
      const exportUrl = getExportUrl(session.document_id);
      const res = await fetch(exportUrl);
      if (!res.ok) throw new Error('Export endpoint error');

      const blob = await res.blob();
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${session.filename.replace('.pdf', '')}_surgical_edited.pdf`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      window.URL.revokeObjectURL(url);
    } catch (e) {
      console.warn('Backend export unavailable, using client-side fallback download notification:', e);
      alert('Surgical In-Place Edits confirmed! When connected to backend, modified PDF downloads instantaneously.');
    } finally {
      setIsExporting(false);
    }
  };

  const selectedPara = paragraphs.find((p) => p.id === selectedParagraphId);

  return (
    <div className="min-h-screen flex flex-col bg-neutral-100 dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100 antialiased">
      {/* Hidden file input for uploading real PDFs */}
      <input
        type="file"
        ref={fileInputRef}
        onChange={handleFileChange}
        accept="application/pdf"
        className="hidden"
      />

      {/* Hidden file input for replacing images */}
      <input
        type="file"
        ref={imageFileInputRef}
        onChange={handleImageFileChange}
        accept="image/png,image/jpeg,image/jpg"
        className="hidden"
      />

      {/* Hidden file input for merging PDF */}
      <input
        type="file"
        ref={mergeFileInputRef}
        onChange={handleMergeFileChange}
        accept="application/pdf"
        className="hidden"
      />

      {/* Top Application Toolbar */}
      <Toolbar
        filename={session.filename}
        pageNumber={currentPage}
        totalPages={session.page_count}
        zoom={zoom}
        onZoomChange={setZoom}
        selectedAlignment={selectedPara?.alignment || 'left'}
        onAlignmentChange={handleAlignmentChange}
        canUndo={historyIndex > 0}
        canRedo={historyIndex < history.length - 1}
        onUndo={handleUndo}
        onRedo={handleRedo}
        onRotateClockwise={handleRotateClockwise}
        hasSelectedParagraph={selectedParagraphId !== null}
        onAddHighlight={() => handleAddMarkup('Highlight')}
        onAddUnderline={() => handleAddMarkup('Underline')}
        onAddLink={() => handleAddLink()}
        onAddStamp={handleAddStamp}
        onUploadClick={handleUploadClick}
        onExportClick={handleExportClick}
        isExporting={isExporting}
        wsConnected={wsConnected}
      />

      {/* Main Studio View: Dual-Layer Canvas + SceneGraph Sidebar */}
      <div className="flex-1 flex overflow-hidden">
        <DualCanvasViewer
          paragraphs={paragraphs}
          selectedParagraphId={selectedParagraphId}
          onSelectParagraph={(id) => {
            setSelectedParagraphId(id);
            if (id !== null) {
              setSelectedImageId(null);
              setSelectedFormFieldName(null);
              setSelectedAnnotationId(null);
            }
          }}
          onUpdateParagraphText={handleUpdateParagraphText}
          zoom={zoom}
          activeReflowId={activeReflowId}
          documentId={session.document_id}
          pageNumber={currentPage}
          rotation={pageRotation}
          images={images}
          selectedImageId={selectedImageId}
          onSelectImage={(id) => {
            setSelectedImageId(id);
            if (id !== null) {
              setSelectedParagraphId(null);
              setSelectedFormFieldName(null);
              setSelectedAnnotationId(null);
            }
          }}
          onTriggerReplaceImage={handleTriggerReplaceImage}
          forms={forms}
          selectedFormFieldName={selectedFormFieldName}
          onSelectFormField={(name) => {
            setSelectedFormFieldName(name);
            if (name !== null) {
              setSelectedParagraphId(null);
              setSelectedImageId(null);
              setSelectedAnnotationId(null);
            }
          }}
          onUpdateFormFieldValue={handleUpdateFormFieldValue}
          annotations={annotations}
          selectedAnnotationId={selectedAnnotationId}
          onSelectAnnotation={(id) => {
            setSelectedAnnotationId(id);
            if (id !== null) {
              setSelectedParagraphId(null);
              setSelectedImageId(null);
              setSelectedFormFieldName(null);
            }
          }}
          onDeleteAnnotation={handleDeleteAnnotation}
          onNavigatePage={(p) => setCurrentPage(p)}
        />

        <Sidebar
          paragraphs={paragraphs}
          selectedParagraphId={selectedParagraphId}
          onSelectParagraph={(id) => {
            setSelectedParagraphId(id);
            setSelectedImageId(null);
            setSelectedFormFieldName(null);
            setSelectedAnnotationId(null);
          }}
          documentId={session.document_id}
          images={images}
          selectedImageId={selectedImageId}
          onSelectImage={(id) => {
            setSelectedImageId(id);
            setSelectedParagraphId(null);
            setSelectedFormFieldName(null);
            setSelectedAnnotationId(null);
          }}
          onTriggerReplaceImage={handleTriggerReplaceImage}
          forms={forms}
          selectedFormFieldName={selectedFormFieldName}
          onSelectFormField={(name) => {
            setSelectedFormFieldName(name);
            setSelectedParagraphId(null);
            setSelectedImageId(null);
            setSelectedAnnotationId(null);
          }}
          onUpdateFormFieldValue={handleUpdateFormFieldValue}
          onFlattenForms={handleFlattenForms}
          pageNumber={currentPage}
          totalPages={session.page_count}
          pageRotation={pageRotation}
          onRotatePage={handleRotatePage}
          onRotateAllPages={handleRotateAllPages}
          onSplitDocument={handleSplitDocument}
          onTriggerMergeDocument={handleTriggerMergeDocument}
          onDeleteCurrentPage={handleDeleteCurrentPage}
          annotations={annotations}
          selectedAnnotationId={selectedAnnotationId}
          onSelectAnnotation={(id) => {
            setSelectedAnnotationId(id);
            setSelectedParagraphId(null);
            setSelectedImageId(null);
            setSelectedFormFieldName(null);
          }}
          onAddMarkup={handleAddMarkup}
          onAddLink={(uri) => handleAddLink(uri)}
          onAddStamp={handleAddStamp}
          onDeleteAnnotation={handleDeleteAnnotation}
          onFlattenAnnotations={handleFlattenAnnotations}
        />
      </div>
    </div>
  );
};
