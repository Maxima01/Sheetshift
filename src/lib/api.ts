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
  async convert(input_path: string, output_directory: string | null): Promise<HistoryEntry> {
    requireDesktop();
    return invoke('convert_file', { request: { input_path, output_directory } });
  },
  async openOutput(path: string): Promise<void> { requireDesktop(); return invoke('open_output', { path }); },
};

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : 'Something went wrong. Please try again.';
}
