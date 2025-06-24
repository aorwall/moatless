import { useCallback } from 'react';
import { usePreviewStore } from '../stores/preview-store';
import type { ElementInfo } from '../types';

export function useInspector() {
  const {
    isInspectorActive,
    selectedElement,
    toggleInspector,
    setSelectedElement,
  } = usePreviewStore();

  const handleElementSelected = useCallback(
    (element: ElementInfo | null) => {
      setSelectedElement(element);
      
      // Copy selector to clipboard
      if (element?.selector) {
        navigator.clipboard.writeText(element.selector).catch(() => {
          // Ignore clipboard errors
        });
      }
    },
    [setSelectedElement]
  );

  const clearSelection = useCallback(() => {
    setSelectedElement(null);
  }, [setSelectedElement]);

  return {
    isActive: isInspectorActive,
    selectedElement,
    toggle: toggleInspector,
    selectElement: handleElementSelected,
    clearSelection,
  };
}