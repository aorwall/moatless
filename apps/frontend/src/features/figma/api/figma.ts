import type {
  FigmaImportDetailResponse,
  FigmaImportResponse,
  ParseClipboardResponse,
} from '@moatless/api-client'
import { unwrapResponse } from '@moatless/api-client/custom-instance'
import {
  getGetImportImageUrl,
  getImport as getImportHandler,
  importClipboard as importClipboardHandler,
  importNode as importNodeHandler,
  parseClipboard as parseClipboardHandler,
} from '@moatless/api-client/sandbox/figma/figma'

export const figmaApi = {
  async importNode(url: string, includeImageInPrompt = false): Promise<FigmaImportResponse> {
    const result = await unwrapResponse(importNodeHandler({ url, includeImageInPrompt }))
    return result as FigmaImportResponse
  },

  async importClipboard(
    clipboardHtml: string,
    includeImageInPrompt = false,
  ): Promise<FigmaImportResponse> {
    const result = await unwrapResponse(
      importClipboardHandler({ clipboardHtml, includeImageInPrompt }),
    )
    return result as FigmaImportResponse
  },

  async parseClipboard(clipboardHtml: string): Promise<ParseClipboardResponse> {
    const result = await unwrapResponse(parseClipboardHandler({ clipboardHtml }))
    return result as ParseClipboardResponse
  },

  async getFigmaImport(importId: string): Promise<FigmaImportDetailResponse> {
    const result = await unwrapResponse(getImportHandler(importId))
    return result as FigmaImportDetailResponse
  },

  getImageUrl(importId: string): string {
    return getGetImportImageUrl(importId)
  },
}

// Export types for use in hooks
export type { FigmaImportResponse, ParseClipboardResponse }
