import { invoke, isTauri } from '@tauri-apps/api/core';

export const TERMS_VERSION = '2026-10-15';
export const SUPPORTED_FORMATS = ['docx', 'pdf', 'xlsx', 'xls', 'xlsb', 'xlsm', 'ods', 'csv', 'tsv', 'txt', 'json'];
export const desktop = isTauri();

export interface Settings {
  output_directory: string | null;
  remember_history: boolean;
  accepted_terms_version: string | null;
}

export interface HistoryEntry {
  id: string;
  input_name: string;
  input_path: string;
  output_path: string | null;
  format: string;
  status: 'success' | 'failed';
  created_at: string;
  rows: number;
  sheets: number;
  duration_ms: number;
  warnings: string[];
  error: string | null;
}

export type PdfReviewMode = 'selected_pages' | 'detect_tables';

export interface PdfReviewOptions {
  mode: PdfReviewMode;
  page_selection: string | null;
}

export type PreviewCell = string | number | boolean | null;

export interface PreviewSheet {
  name: string;
  rows: PreviewCell[][];
  total_rows: number;
  total_columns: number;
  truncated: boolean;
}

export interface ConversionPreview {
  input_name: string;
  format: string;
  source_fingerprint: string;
  page_count: number | null;
  selected_pages: number[];
  detected_tables: number;
  sheets: PreviewSheet[];
  warnings: string[];
}

export interface CellEdit {
  sheet_index: number;
  row_index: number;
  column_index: number;
  value: string;
}

export const defaultSettings: Settings = {
  output_directory: null,
  remember_history: true,
  accepted_terms_version: null,
};

function requireDesktop() {
  if (!desktop) throw new Error('Open Sheetshift in the desktop app to work with local files.');
}

export const api = {
  async settings(): Promise<Settings> { requireDesktop(); return invoke('get_settings'); },
  async saveSettings(settings: Settings): Promise<Settings> {
    requireDesktop();
    return invoke('save_settings', { settings });
  },
  async history(): Promise<HistoryEntry[]> { requireDesktop(); return invoke('get_history', { limit: 1000 }); },
  async clearHistory(): Promise<void> { requireDesktop(); return invoke('clear_history'); },
  async acceptTerms(): Promise<Settings> { requireDesktop(); return invoke('accept_terms'); },
  async formats(): Promise<string[]> { requireDesktop(); return invoke('get_supported_formats'); },
  async convert(input_path: string, output_directory: string | null, preserve_formatting = false): Promise<HistoryEntry> {
    requireDesktop();
    return invoke('convert_file', { request: { input_path, output_directory, preserve_formatting } });
  },
  async preview(input_path: string, pdf_options: PdfReviewOptions): Promise<ConversionPreview> {
    requireDesktop();
    return invoke('preview_file', { request: { input_path, pdf_options } });
  },
  async convertReviewed(
    input_path: string,
    output_directory: string | null,
    pdf_options: PdfReviewOptions,
    source_fingerprint: string,
    edits: CellEdit[],
    sheet_names: string[],
    preserve_formatting = false,
  ): Promise<HistoryEntry> {
    requireDesktop();
    return invoke('convert_reviewed_file', {
      request: { input_path, output_directory, pdf_options, source_fingerprint, edits, sheet_names, preserve_formatting },
    });
  },
  async openOutput(path: string): Promise<void> { requireDesktop(); return invoke('open_output', { path }); },
};

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : 'Something went wrong. Please try again.';
}
