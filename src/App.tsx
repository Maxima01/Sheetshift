import { useCallback, useEffect, useRef, useState } from 'react';
import {
  AlertCircle, ArrowDownToLine, ArrowRight, Check, CheckCircle2, ChevronRight,
  Clock3, Eye, FileJson2, FileSpreadsheet, FileText, Files, Folder, FolderOpen,
  History, Info, LoaderCircle, Monitor, Plus, RefreshCw, ScanSearch, Search,
  Settings2, ShieldCheck, Table2, Trash2, X,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { api, defaultSettings, desktop, errorMessage, SUPPORTED_FORMATS, TERMS_VERSION } from './lib/api';
import type { CellEdit, ConversionPreview, HistoryEntry, PdfReviewOptions, Settings } from './lib/api';
import termsText from '../docs/TERMS.md?raw';

type Page = 'convert' | 'history' | 'settings' | 'about' | 'terms';
type QueueItem = {
  id: string;
  path: string;
  name: string;
  format: string;
  status: 'waiting' | 'converting' | 'success' | 'failed';
  result?: HistoryEntry;
  error?: string;
  pdfOptions: PdfReviewOptions;
  preview?: ConversionPreview;
  edits: CellEdit[];
  sheetNames: string[];
  reviewConfirmed: boolean;
  formatProtection: boolean;
};

const defaultPdfOptions: PdfReviewOptions = { mode: 'detect_tables', page_selection: null };

const navItems: { id: Page; label: string; icon: LucideIcon }[] = [
  { id: 'convert', label: 'Convert files', icon: Files },
  { id: 'history', label: 'History', icon: History },
  { id: 'settings', label: 'Settings', icon: Settings2 },
];

function filename(path: string) { return path.split(/[\\/]/).pop() || path; }
function dateLabel(date: string) {
  const parsed = new Date(date);
  return Number.isNaN(parsed.getTime()) ? date : new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(parsed);
}
function formatIcon(format: string): LucideIcon {
  return ['xlsx', 'xls', 'xlsm', 'xlsb', 'ods', 'csv', 'tsv'].includes(format) ? FileSpreadsheet : format === 'json' ? FileJson2 : FileText;
}

function FileBadge({ format }: { format: string }) {
  const Icon = formatIcon(format);
  return <span className={`file-badge file-badge-${format === 'pdf' ? 'pdf' : Icon === FileSpreadsheet ? 'sheet' : 'doc'}`} aria-hidden="true"><Icon size={22} strokeWidth={1.7} /></span>;
}

function TermsContent() {
  return <div className="terms-copy">{termsText.split(/\n\s*\n/).map((block, index) => {
    if (block.startsWith('# ')) return null;
    if (block.startsWith('## ')) return <h3 key={index}>{block.replace(/^## /, '')}</h3>;
    if (block.startsWith('- ')) return <ul key={index}>{block.split('\n').map((line, i) => <li key={i}>{line.replace(/^- /, '').replace(/\*\*/g, '')}</li>)}</ul>;
    return <p key={index}>{block.replace(/\*\*/g, '')}</p>;
  })}</div>;
}

function Modal({ title, children, onClose, error, className = '' }: { title: string; children: React.ReactNode; onClose: () => void; error?: string | null; className?: string }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement as HTMLElement | null;
    dialog?.showModal();
    const cancel = (event: Event) => { event.preventDefault(); closeRef.current(); };
    dialog?.addEventListener('cancel', cancel);
    return () => {
      dialog?.removeEventListener('cancel', cancel);
      dialog?.close();
      previousFocus?.focus();
    };
  }, []);
  return <dialog ref={dialogRef} className={`dialog ${className}`} aria-labelledby="dialog-heading">
    <div className="dialog-heading"><h2 id="dialog-heading">{title}</h2><button className="icon-button" onClick={onClose} aria-label="Close dialog"><X size={20} /></button></div>
    {error && <div className="alert alert-error" role="alert"><AlertCircle size={19} /><span>{error}</span></div>}
    {children}
  </dialog>;
}

export default function App() {
  const [page, setPage] = useState<Page>('convert');
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [formats, setFormats] = useState(SUPPORTED_FORMATS);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [ready, setReady] = useState(!desktop);
  const [loading, setLoading] = useState(desktop);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [termsOpen, setTermsOpen] = useState(false);
  const [termsChecked, setTermsChecked] = useState(false);
  const [pendingConversion, setPendingConversion] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [dialogError, setDialogError] = useState<string | null>(null);
  const [reviewId, setReviewId] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [activeSheet, setActiveSheet] = useState(0);
  const [search, setSearch] = useState('');
  const [historyFilter, setHistoryFilter] = useState<'all' | 'success' | 'failed'>('all');
  const stopRequested = useRef(false);
  const running = useRef(false);
  const localWriteVersion = useRef(0);
  const canUseFiles = desktop && ready && !busy && !saving;
  const waiting = queue.filter(item => item.status === 'waiting');
  const completed = queue.filter(item => item.status === 'success' || item.status === 'failed');
  const successful = queue.filter(item => item.status === 'success');
  const activeReview = queue.find(item => item.id === reviewId);
  const pdfsNeedingReview = waiting.filter(item => item.format === 'pdf' && !item.reviewConfirmed);

  const initialize = useCallback(async () => {
    if (!desktop) return;
    setLoading(true);
    setError(null);
    const results = await Promise.allSettled([api.settings(), api.history(), api.formats()]);
    if (results[0].status === 'fulfilled') { setSettings(results[0].value); setReady(true); }
    if (results[1].status === 'fulfilled') setHistory(results[1].value);
    if (results[2].status === 'fulfilled') setFormats(results[2].value);
    const failed = results.find(result => result.status === 'rejected');
    if (failed?.status === 'rejected') setError(`Could not load local data. ${errorMessage(failed.reason)}`);
    setLoading(false);
  }, []);

  useEffect(() => { void initialize(); }, [initialize]);

  useEffect(() => {
    if (!desktop || !ready || busy || saving) return;
    let disposed = false;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing || running.current) return;
      refreshing = true;
      const version = localWriteVersion.current;
      try {
        const results = await Promise.allSettled([api.settings(), api.history()]);
        // A local write or navigation can make an earlier read stale.
        if (disposed || running.current || version !== localWriteVersion.current) return;
        if (results[0].status === 'fulfilled') setSettings(results[0].value);
        if (results[1].status === 'fulfilled') setHistory(results[1].value);
        const failed = results.find(result => result.status === 'rejected');
        if (failed?.status === 'rejected') setError(`Could not refresh local data. ${errorMessage(failed.reason)}`);
      } finally { refreshing = false; }
    };
    const onFocus = () => { void refresh(); };
    window.addEventListener('focus', onFocus);
    if (page === 'history' || page === 'settings') void refresh();
    return () => { disposed = true; window.removeEventListener('focus', onFocus); };
  }, [page, ready, busy, saving]);

  const addPaths = useCallback((paths: string[]) => {
    if (running.current) return;
    const rejected: string[] = [];
    const accepted = paths.flatMap(path => {
      const name = filename(path);
      const format = name.includes('.') ? name.split('.').pop()!.toLowerCase() : '';
      if (!formats.includes(format)) { rejected.push(name); return []; }
      return [{
        id: crypto.randomUUID(), path, name, format, status: 'waiting' as const,
        pdfOptions: { ...defaultPdfOptions }, edits: [], sheetNames: [], reviewConfirmed: false, formatProtection: false,
      }];
    });
    setQueue(current => {
      const existing = new Set(current.map(item => item.path));
      return [...current, ...accepted.filter(item => {
        if (existing.has(item.path)) return false;
        existing.add(item.path);
        return true;
      })];
    });
    if (rejected.length) setError(`Unsupported ${rejected.length === 1 ? 'file' : 'files'}: ${rejected.join(', ')}. Choose one of the supported formats below.`);
    else setError(null);
    setNotice(null);
    setPage('convert');
  }, [formats]);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void import('@tauri-apps/api/webviewWindow').then(async ({ getCurrentWebviewWindow }) => {
      const remove = await getCurrentWebviewWindow().onDragDropEvent(event => {
        if (event.payload.type === 'enter' || event.payload.type === 'over') setIsDragging(true);
        else if (event.payload.type === 'leave') setIsDragging(false);
        else if (event.payload.type === 'drop') {
          setIsDragging(false);
          if (ready && !running.current) addPaths(event.payload.paths);
        }
      });
      if (disposed) remove(); else unlisten = remove;
    }).catch(cause => setError(`Drag and drop is unavailable. Use Add files instead. ${errorMessage(cause)}`));
    return () => { disposed = true; unlisten?.(); };
  }, [addPaths, ready]);

  const chooseFiles = async () => {
    if (!canUseFiles) return;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const paths = await open({ multiple: true, directory: false, filters: [{ name: 'Supported documents', extensions: formats }] });
      if (paths) addPaths(Array.isArray(paths) ? paths : [paths]);
    } catch (cause) { setError(`Could not select files. ${errorMessage(cause)}`); }
  };

  const saveSettings = async (next: Settings, message = 'Settings saved.') => {
    localWriteVersion.current++;
    setSaving(true);
    setError(null);
    try {
      const saved = await api.saveSettings(next);
      setSettings(saved);
      setNotice(message);
    } catch (cause) { setError(`Could not save settings. ${errorMessage(cause)}`); }
    finally { setSaving(false); }
  };

  const chooseFolder = async () => {
    if (!canUseFiles) return;
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const path = await open({ directory: true, multiple: false, defaultPath: settings.output_directory || undefined, title: 'Choose where Excel files are saved' });
      if (typeof path === 'string') await saveSettings({ ...settings, output_directory: path }, 'Output folder saved.');
    } catch (cause) { setError(`Could not select a folder. ${errorMessage(cause)}`); }
  };

  const openOutput = async (path: string) => {
    try { await api.openOutput(path); }
    catch (cause) { setError(`Could not open the workbook. ${errorMessage(cause)}`); }
  };

  const preparePreview = async (item: QueueItem, options = item.pdfOptions) => {
    if (!desktop || previewLoading || busy) return;
    setPreviewLoading(true);
    setPreviewError(null);
    try {
      const preview = await api.preview(item.path, options);
      setQueue(current => current.map(entry => entry.id === item.id ? {
        ...entry,
        pdfOptions: options,
        preview,
        edits: [],
        sheetNames: preview.sheets.map(sheet => sheet.name),
        reviewConfirmed: false,
        error: undefined,
      } : entry));
      setActiveSheet(0);
    } catch (cause) {
      setPreviewError(errorMessage(cause));
    } finally {
      setPreviewLoading(false);
    }
  };

  const openReview = (item: QueueItem) => {
    setReviewId(item.id);
    setActiveSheet(0);
    setPreviewError(null);
    if (!item.preview) void preparePreview(item);
  };

  const updatePdfOptions = (id: string, next: PdfReviewOptions) => {
    setQueue(current => current.map(item => item.id === id ? {
      ...item,
      pdfOptions: next,
      preview: undefined,
      edits: [],
      sheetNames: [],
      reviewConfirmed: false,
    } : item));
    setActiveSheet(0);
    setPreviewError(null);
  };

  const updatePreviewCell = (itemId: string, sheetIndex: number, rowIndex: number, columnIndex: number, value: string) => {
    setQueue(current => current.map(item => {
      if (item.id !== itemId || !item.preview) return item;
      const sheets = item.preview.sheets.map((sheet, index) => index !== sheetIndex ? sheet : {
        ...sheet,
        rows: sheet.rows.map((row, index) => index !== rowIndex ? row : row.map((cell, index) => index === columnIndex ? value : cell)),
      });
      const edits = item.edits.filter(edit => !(edit.sheet_index === sheetIndex && edit.row_index === rowIndex && edit.column_index === columnIndex));
      edits.push({ sheet_index: sheetIndex, row_index: rowIndex, column_index: columnIndex, value });
      return { ...item, preview: { ...item.preview, sheets }, edits, reviewConfirmed: false, formatProtection: false };
    }));
  };

  const updateSheetName = (itemId: string, sheetIndex: number, value: string) => {
    setQueue(current => current.map(item => {
      if (item.id !== itemId) return item;
      const sheetNames = [...item.sheetNames];
      sheetNames[sheetIndex] = value;
      return { ...item, sheetNames, reviewConfirmed: false, formatProtection: value === item.preview?.sheets[sheetIndex]?.name ? item.formatProtection : false };
    }));
  };

  const runConversion = async (activeSettings = settings) => {
    if (!desktop || !ready || running.current || !waiting.length) return;
    const reviewRequired = waiting.find(item => item.format === 'pdf' && !item.reviewConfirmed);
    if (reviewRequired) {
      setNotice('Review each PDF and confirm its pages or detected tables before conversion.');
      openReview(reviewRequired);
      return;
    }
    const emptyReview = waiting.find(item => item.preview && item.preview.sheets.length === 0);
    if (emptyReview) {
      setNotice('No table candidates are ready to export. Adjust the review options for this file.');
      openReview(emptyReview);
      return;
    }
    if (activeSettings.accepted_terms_version !== TERMS_VERSION) {
      setPendingConversion(true);
      setTermsChecked(false);
      setDialogError(null);
      setTermsOpen(true);
      return;
    }
    running.current = true;
    localWriteVersion.current++;
    stopRequested.current = false;
    setBusy(true);
    setError(null);
    setNotice(null);
    let succeeded = 0;
    let failed = 0;
    try {
      for (const item of waiting) {
        if (stopRequested.current) break;
        setQueue(current => current.map(entry => entry.id === item.id ? { ...entry, status: 'converting', error: undefined } : entry));
        try {
          const result = item.preview && item.reviewConfirmed
            ? await api.convertReviewed(item.path, activeSettings.output_directory, item.pdfOptions, item.preview.source_fingerprint, item.edits, item.sheetNames, item.formatProtection)
            : await api.convert(item.path, activeSettings.output_directory, item.formatProtection);
          if (result.status === 'success') succeeded++; else failed++;
          setQueue(current => current.map(entry => entry.id === item.id ? { ...entry, status: result.status, result, error: result.error ?? undefined } : entry));
        } catch (cause) {
          failed++;
          setQueue(current => current.map(entry => entry.id === item.id ? { ...entry, status: 'failed', error: errorMessage(cause) } : entry));
        }
      }
      try { setHistory(await api.history()); }
      catch (cause) { setError(`Conversion finished, but history could not be refreshed. ${errorMessage(cause)}`); }
      setNotice(`${stopRequested.current ? 'Queue stopped. ' : ''}${succeeded} ${succeeded === 1 ? 'workbook' : 'workbooks'} created${failed ? `, ${failed} ${failed === 1 ? 'file' : 'files'} failed` : ''}.`);
    } finally {
      running.current = false;
      setBusy(false);
    }
  };

  const acceptTerms = async () => {
    if (!termsChecked || saving) return;
    localWriteVersion.current++;
    setDialogError(null);
    setSaving(true);
    try {
      const saved = await api.acceptTerms();
      setSettings(saved);
      setTermsOpen(false);
      setNotice('Terms accepted on this device.');
      if (pendingConversion) { setPendingConversion(false); void runConversion(saved); }
    } catch (cause) { setDialogError(`Could not save acceptance. ${errorMessage(cause)}`); }
    finally { setSaving(false); }
  };

  const clearHistory = async () => {
    localWriteVersion.current++;
    setDialogError(null);
    setSaving(true);
    try {
      await api.clearHistory();
      setHistory([]);
      setClearOpen(false);
      setNotice('Conversion history cleared. Your files are unchanged.');
    } catch (cause) { setDialogError(`Could not clear history. ${errorMessage(cause)}`); }
    finally { setSaving(false); }
  };

  const filteredHistory = history.filter(entry => (historyFilter === 'all' || entry.status === historyFilter) && `${entry.input_name} ${entry.input_path} ${entry.output_path ?? ''} ${entry.format}`.toLowerCase().includes(search.toLowerCase()));
  const pageLabel = page === 'convert' ? 'Convert to Excel' : page === 'history' ? 'Conversion history' : page === 'settings' ? 'Settings' : page === 'about' ? 'About Sheetshift' : 'Terms and conditions';

  return <div className="app-shell">
    <a className="skip-link" href="#main-content">Skip to content</a>
    <aside className="sidebar">
      <a href="#" className="brand" aria-label="Sheetshift home" onClick={event => { event.preventDefault(); setPage('convert'); }}>
        <span className="brand-mark"><FileSpreadsheet size={25} strokeWidth={1.65} /></span>
        <span>Sheetshift<span className="brand-caption">Documents to Excel</span></span>
      </a>
      <nav className="main-nav" aria-label="Main navigation">{navItems.map(({ id, label, icon: Icon }) =>
        <button key={id} className={`nav-item ${page === id ? 'active' : ''}`} aria-current={page === id ? 'page' : undefined} onClick={() => setPage(id)}>
          <Icon size={19} strokeWidth={1.8} /><span>{label}</span>{id === 'convert' && queue.length > 0 && <span className="nav-count">{queue.length}</span>}
        </button>,
      )}</nav>
      <div className="sidebar-bottom">
        <div className="local-note"><ShieldCheck size={19} /><div><strong>On your device</strong><span>Your documents stay local.</span></div></div>
        <nav aria-label="Information" className="secondary-nav">
          <button className={`nav-item ${page === 'about' ? 'active' : ''}`} onClick={() => setPage('about')} aria-current={page === 'about' ? 'page' : undefined}><Info size={18} /><span>About Sheetshift</span></button>
          <button className={`nav-item ${page === 'terms' ? 'active' : ''}`} onClick={() => setPage('terms')} aria-current={page === 'terms' ? 'page' : undefined}><FileText size={18} /><span>Terms and conditions</span></button>
        </nav>
        <div className="sidebar-version"><span>Sheetshift 0.1.0</span><span className="system-theme"><Monitor size={13} /> System theme</span></div>
      </div>
    </aside>

    <div className="workspace">
      <header className="topbar"><div className="breadcrumb"><span>Workspace</span><ChevronRight size={14} /><strong>{pageLabel}</strong></div><span className="runtime-label"><span className={`runtime-dot ${desktop ? '' : 'preview'}`} />{desktop ? 'Local workspace' : 'Browser preview'}</span></header>
      <main id="main-content" className={`main-content page-${page}`}>
        {error && <div className="alert alert-error" role="alert"><AlertCircle size={19} /><div>{error}{!ready && desktop && <button className="text-button" onClick={() => void initialize()} disabled={loading}>{loading ? 'Loading…' : 'Try loading again'}</button>}</div><button className="icon-button" onClick={() => setError(null)} aria-label="Dismiss error"><X size={17} /></button></div>}
        {notice && <div className="alert alert-notice" role="status"><CheckCircle2 size={18} /><span>{notice}</span><button className="icon-button" onClick={() => setNotice(null)} aria-label="Dismiss notification"><X size={17} /></button></div>}

        {page === 'convert' && <>
          <div className="page-heading"><div><h1>A fresh start for your files.</h1><p>Turn documents, text, and tables into Excel workbooks.</p></div><span className="output-chip"><FileSpreadsheet size={16} /> Excel .xlsx</span></div>
          {!desktop && <div className="preview-notice"><Monitor size={19} /><div><strong>You’re viewing the desktop interface.</strong><span>Open the Tauri app to select local files and convert them. No files are processed in this preview.</span></div></div>}

          <section className={`dropzone ${isDragging ? 'dragging' : ''}`} aria-label="Add files for conversion" onDragOver={event => { event.preventDefault(); if (desktop) setIsDragging(true); }} onDragLeave={() => setIsDragging(false)} onDrop={event => { event.preventDefault(); setIsDragging(false); if (!desktop) setError('File conversion is available in the Sheetshift desktop app. Start the desktop app to add files.'); }}>
            <div className="conversion-visual" aria-hidden="true"><span className="source-doc"><FileText size={28} strokeWidth={1.4} /><span>DOC</span></span><span className="source-pdf"><FileText size={28} strokeWidth={1.4} /><span>PDF</span></span><ArrowRight className="visual-arrow" size={22} strokeWidth={1.6} /><span className="destination-doc"><FileSpreadsheet size={38} strokeWidth={1.4} /><span>XLSX</span></span></div>
            <h2>{isDragging ? 'Drop to add your files' : 'Bring your files. Keep the useful parts.'}</h2>
            <p>Drop documents here, or choose them from your computer.</p>
            <button className="button button-primary" onClick={() => void chooseFiles()} disabled={!canUseFiles}><Plus size={18} />{loading ? 'Loading workspace…' : 'Choose files'}</button>
            <span className="drop-hint">Multiple files welcome. Originals stay unchanged.</span>
          </section>

          <div className="format-strip" aria-label="Supported input formats"><span className="format-strip-label">Works with</span><div><FileText size={16} /><span>PDF, DOCX, TXT</span></div><div><FileSpreadsheet size={16} /><span>XLSX, XLS, XLSB, XLSM, ODS</span></div><div><FileJson2 size={16} /><span>CSV, TSV, JSON</span></div></div>

          <section className="conversion-panel" aria-labelledby="queue-heading">
            <div className="panel-heading"><div className="inline-heading"><h2 id="queue-heading">Conversion queue</h2><span className="count-badge">{queue.length}</span></div>{queue.length > 0 && <button className="button button-small button-quiet" onClick={() => void chooseFiles()} disabled={!canUseFiles}><Plus size={15} /> Add files</button>}</div>
            {queue.length === 0 ? <div className="queue-empty"><span className="empty-line-icon"><Files size={24} strokeWidth={1.5} /></span><div><strong>Your next workbook starts here.</strong><p>Add files above to review and convert them together.</p></div></div> : <div className="queue-list">{queue.map(item => <article className={`queue-item queue-${item.status}`} key={item.id}>
              <FileBadge format={item.format} />
              <div className="file-details">
                <strong title={item.path}>{item.name}</strong>
                <span className="file-meta">{item.format.toUpperCase()}{item.result?.status === 'success'
                  ? ` · ${item.result.rows.toLocaleString()} rows · ${item.result.sheets} ${item.result.sheets === 1 ? 'sheet' : 'sheets'}`
                  : item.status === 'waiting' && item.preview
                    ? ` · ${item.reviewConfirmed ? 'Review confirmed' : 'Preview generated'} · ${item.preview.sheets.length} ${item.preview.sheets.length === 1 ? 'sheet' : 'sheets'}`
                    : item.status === 'waiting' && item.format === 'pdf'
                      ? ' · Review required'
                      : item.status === 'waiting' ? ' · Ready to convert' : item.status === 'converting' ? ' · Applying reviewed content…' : ''}</span>
                {item.preview && item.status === 'waiting' && <span className={`review-summary ${item.reviewConfirmed ? '' : 'pending'}`}>{item.reviewConfirmed ? <CheckCircle2 size={13} /> : <Info size={13} />}{item.reviewConfirmed ? (item.edits.length ? `${item.edits.length} ${item.edits.length === 1 ? 'correction' : 'corrections'} saved` : 'Reviewed with no corrections') : 'Open the preview and confirm it before conversion'}</span>}
                {item.error && <p className="item-error">{item.error}</p>}
                {item.result?.warnings?.map((warning, index) => <p className="item-warning" key={index}><AlertCircle size={13} />{warning}</p>)}
                {item.result?.output_path && <span className="output-path" title={item.result.output_path}>{item.result.output_path}</span>}
              </div>
              <div className="queue-item-actions">{item.status === 'converting' ? <span className="status status-working"><LoaderCircle size={15} className="spin" /> Converting</span> : item.status === 'success' ? <><span className="status status-success"><Check size={15} /> Done</span>{item.result?.output_path && <button className="button button-small button-quiet" onClick={() => void openOutput(item.result!.output_path!)}><FolderOpen size={15} />Open</button>}</> : item.status === 'failed' ? <><span className="status status-failed">Failed</span><button className="text-button" disabled={busy} onClick={() => setQueue(current => current.map(entry => entry.id === item.id ? { ...entry, status: 'waiting', result: undefined, error: undefined } : entry))}>Retry</button></> : <button className={`button button-small ${item.format === 'pdf' && !item.reviewConfirmed ? 'button-review' : 'button-quiet'}`} disabled={busy} onClick={() => openReview(item)}><Eye size={15} />{item.preview ? item.reviewConfirmed ? 'Review again' : 'Finish review' : item.format === 'pdf' ? 'Review PDF' : 'Preview'}</button>}{item.status !== 'converting' && <button className="icon-button remove-file" aria-label={`Remove ${item.name} from queue`} disabled={busy} onClick={() => setQueue(current => current.filter(entry => entry.id !== item.id))}><X size={16} /></button>}</div>
            </article>)}</div>}
            <div className="destination-row"><Folder size={19} className="muted-icon" /><div className="destination-copy"><span>Save workbooks to</span><strong title={settings.output_directory || undefined}>{settings.output_directory || 'Same folder as each source file'}</strong></div><button className="button button-small button-quiet" onClick={() => void chooseFolder()} disabled={!canUseFiles}>Change folder</button></div>
            <div className="conversion-footer"><div className="footer-state">{busy ? <><LoaderCircle size={16} className="spin" /><span>Converting locally. {completed.length} of {queue.length} finished.</span></> : queue.length ? <span>{pdfsNeedingReview.length ? `${pdfsNeedingReview.length} ${pdfsNeedingReview.length === 1 ? 'PDF needs' : 'PDFs need'} review before conversion` : waiting.length ? `${waiting.length} ${waiting.length === 1 ? 'file' : 'files'} ready to convert` : `${successful.length} ${successful.length === 1 ? 'workbook' : 'workbooks'} created`}</span> : <><ShieldCheck size={16} /><span>Local processing. No uploads.</span></>}</div><div className="footer-actions">{!busy && completed.length > 0 && <button className="text-button" onClick={() => setQueue(current => current.filter(item => item.status === 'waiting'))}>Clear finished</button>}{busy ? <button className="button button-secondary" onClick={() => { stopRequested.current = true; setNotice('Stopping after the current file finishes.'); }}>Stop after this file</button> : <button className="button button-primary convert-button" disabled={!canUseFiles || waiting.length === 0} onClick={() => void runConversion()}>{pdfsNeedingReview.length ? <Eye size={18} /> : <ArrowDownToLine size={18} />}{pdfsNeedingReview.length ? 'Review PDFs' : 'Convert to Excel'}{waiting.length > 0 && <span className="button-count">{waiting.length}</span>}</button>}</div></div>
          </section>
          <p className="extraction-note"><Info size={15} /><span>Preview and correct extracted cells before export. PDF page selection reads only the pages you request; image-only pages still need OCR.</span></p>
        </>}

        {page === 'history' && <>
          <div className="page-heading"><div><h1>Conversion history</h1><p>Find the files you’ve worked with, and pick up where you left off.</p></div><button className="button button-secondary" disabled={!desktop || !history.length || busy || saving} onClick={() => { setDialogError(null); setClearOpen(true); }}><Trash2 size={16} />Clear history</button></div>
          <div className="history-toolbar"><label className="search-field"><Search size={18} /><input value={search} onChange={event => setSearch(event.target.value)} placeholder="Search file names, formats, or folders" aria-label="Search conversion history" />{search && <button className="icon-button" onClick={() => setSearch('')} aria-label="Clear search"><X size={16} /></button>}</label><label className="filter-field"><span>Status</span><select value={historyFilter} onChange={event => setHistoryFilter(event.target.value as typeof historyFilter)}><option value="all">All results</option><option value="success">Successful</option><option value="failed">Failed</option></select></label></div>
          {!settings.remember_history && <div className="inline-note"><Info size={16} />New conversions aren’t being saved to history. You can turn this on in Settings.</div>}
          {filteredHistory.length === 0 ? <div className="large-empty"><History size={36} strokeWidth={1.4} /><h2>{search || historyFilter !== 'all' ? 'No matching conversions' : 'A place for your past work.'}</h2><p>{search || historyFilter !== 'all' ? 'Try a different file name or choose all results.' : desktop ? 'Your saved conversion results will appear here after your first run.' : 'Conversion history is stored locally by the desktop app.'}</p>{search || historyFilter !== 'all' ? <button className="button button-secondary" onClick={() => { setSearch(''); setHistoryFilter('all'); }}>Reset filters</button> : <button className="button button-secondary" onClick={() => setPage('convert')}>Go to conversion</button>}</div> : <><div className="history-list">{filteredHistory.map(entry => <article className="history-item" key={entry.id}>
            <FileBadge format={entry.format} /><div className="file-details"><strong title={entry.input_path}>{entry.input_name}</strong><span className="file-meta">{dateLabel(entry.created_at)}{entry.status === 'success' ? ` · ${entry.rows.toLocaleString()} rows` : ''}</span><span className="output-path" title={entry.input_path}>{entry.input_path}</span>{entry.error && <p className="item-error">{entry.error}</p>}{entry.warnings.map((warning, index) => <p key={index} className="item-warning"><AlertCircle size={13} />{warning}</p>)}</div><div className="history-item-actions"><span className={`status status-${entry.status === 'success' ? 'success' : 'failed'}`}>{entry.status === 'success' ? <><Check size={14} /> Converted</> : 'Failed'}</span>{entry.output_path && <button className="button button-small button-quiet" onClick={() => void openOutput(entry.output_path!)}><FolderOpen size={15} />Open workbook</button>}</div>
          </article>)}</div><p className="section-footnote">Showing {filteredHistory.length} of {history.length} recent records. History contains file paths and results, not copies of your documents.</p></>}
        </>}

        {page === 'settings' && <>
          <div className="page-heading"><div><h1>Make yourself at home.</h1><p>A few preferences for your local workspace.</p></div></div>
          <section className="settings-section"><h2>Files and history</h2><div className="settings-row"><div><h3>Default output folder</h3><p>Choose where new Excel workbooks are saved.</p><span className="setting-path" title={settings.output_directory || undefined}><Folder size={16} />{settings.output_directory || 'Same folder as each source file'}</span></div><div className="setting-actions"><button className="button button-secondary" disabled={!canUseFiles} onClick={() => void chooseFolder()}>Choose folder</button>{settings.output_directory && <button className="text-button" disabled={!canUseFiles} onClick={() => void saveSettings({ ...settings, output_directory: null }, 'Output folder reset to each source file’s folder.')}>Reset to source folder</button>}</div></div><div className="settings-row"><div><h3>Remember conversion history</h3><p>Save file paths, results, and timestamps on this device. Turning this off keeps existing records.</p></div><label className="switch"><input type="checkbox" checked={settings.remember_history} disabled={!canUseFiles} onChange={event => void saveSettings({ ...settings, remember_history: event.target.checked })} aria-label="Remember conversion history" /><span /></label></div><div className="settings-row"><div><h3>Clear conversion history</h3><p>Remove saved records. Your source files and workbooks stay unchanged.</p></div><button className="button button-secondary" disabled={!desktop || !history.length || busy || saving} onClick={() => { setDialogError(null); setClearOpen(true); }}><Trash2 size={16} />Clear history</button></div></section>
          <section className="settings-section"><h2>Appearance</h2><div className="settings-row"><div><h3>In sync with your system</h3><p>Sheetshift follows your operating system’s light or dark appearance automatically.</p></div><span className="setting-value"><Monitor size={18} />System default</span></div></section>
          <section className="settings-section"><h2>Terms and privacy</h2><div className="settings-row"><div><h3>Terms and conditions</h3><p>{settings.accepted_terms_version === TERMS_VERSION ? `Version ${TERMS_VERSION} accepted on this device.` : 'Review and accept the terms before your first conversion.'}</p></div><button className="button button-secondary" onClick={() => setPage('terms')}>Read terms</button></div><div className="settings-row"><div><h3>Your workspace stays on this device</h3><p>Conversions run locally. Preferences and history are stored in a local database. Paths and records are not encrypted.</p></div><ShieldCheck size={24} className="settings-privacy-icon" /></div></section>
          {!desktop && <p className="section-footnote">Preferences can be changed and saved in the desktop app.</p>}
        </>}

        {page === 'about' && <>
          <div className="page-heading"><div><h1>A clearer path to Excel.</h1><p>Sheetshift turns the content in your files into workbooks you can use.</p></div></div>
          <section className="about-intro"><span className="about-mark"><FileSpreadsheet size={43} strokeWidth={1.4} /></span><div><h2>Sheetshift <span>0.1.0</span></h2><p>A local document converter, with a desktop workspace and a command-line interface powered by the same Rust engine.</p></div></section>
          <section className="read-section"><h2>From documents to workable data</h2><p>Document text becomes spreadsheet rows. DOCX tables and existing spreadsheet data retain rows and columns where possible. PDF conversion extracts text; it does not reconstruct a document’s visual layout.</p><p>Each source produces an XLSX workbook. Review the warnings and resulting workbook before using it for reporting, calculations, or other important work.</p></section>
          <section className="read-section"><h2>What you can bring</h2><div className="supported-format-list">{formats.map(format => <span key={format}>{format.toUpperCase()}</span>)}</div><p>Legacy Word files (.doc), password-protected documents, and image-only PDFs need to be exported or prepared in another application first. Normal conversion extracts values; Format Protection keeps native content for unchanged XLSX files.</p></section>
          <section className="read-section"><h2>Built for a local workflow</h2><p>Your source files stay in place. Conversions and history stay on your device. You choose the output folder and whether conversion records are saved. Generated workbooks can be opened from the queue or history.</p><div className="about-facts"><span><ShieldCheck size={17} />Local processing</span><span><Monitor size={17} />System appearance</span><span><Clock3 size={17} />Saved history</span></div></section>
        </>}

        {page === 'terms' && <>
          <div className="page-heading"><div><h1>Terms and conditions</h1><p>Version {TERMS_VERSION}. Please read before converting your files.</p></div>{settings.accepted_terms_version === TERMS_VERSION && <span className="status status-success"><Check size={15} />Accepted</span>}</div>
          <TermsContent />
          {settings.accepted_terms_version !== TERMS_VERSION && <div className="terms-page-action"><p>Acceptance is stored locally on this device.</p><button className="button button-primary" disabled={!desktop || !ready || busy} onClick={() => { setPendingConversion(false); setTermsChecked(false); setDialogError(null); setTermsOpen(true); }}>Review and accept terms</button></div>}
        </>}
      </main>
      <footer className="workspace-footer"><span><ShieldCheck size={13} />Made for local work</span><div className="footer-links"><button onClick={() => setPage('terms')}>Terms</button><button onClick={() => setPage('about')}>Supported formats and limitations<ChevronRight size={13} /></button></div></footer>
    </div>

    {activeReview && <Modal title={`Review conversion · ${activeReview.name}`} className="review-dialog" error={previewError} onClose={() => { if (!previewLoading) setReviewId(null); }}>
      <div className="review-source"><FileBadge format={activeReview.format} /><div><strong>{activeReview.name}</strong><span>{activeReview.path}</span></div><span className="review-local"><ShieldCheck size={14} />Local preview</span></div>
      {activeReview.format === 'pdf' && <section className="pdf-review-options" aria-labelledby="pdf-review-heading">
        <div className="review-section-heading"><div><h3 id="pdf-review-heading">Choose what to extract</h3><p>Both options reconstruct tables and keep wrapped text together.</p></div>{activeReview.preview?.page_count && <span>{activeReview.preview.page_count} pages</span>}</div>
        <div className="review-mode-grid">
          <button className={`review-mode ${activeReview.pdfOptions.mode === 'selected_pages' ? 'active' : ''}`} aria-pressed={activeReview.pdfOptions.mode === 'selected_pages'} onClick={() => updatePdfOptions(activeReview.id, { ...activeReview.pdfOptions, mode: 'selected_pages' })}><FileText size={20} /><span><strong>Select pages</strong><small>Reconstruct content on the pages you name</small></span>{activeReview.pdfOptions.mode === 'selected_pages' && <Check size={17} />}</button>
          <button className={`review-mode ${activeReview.pdfOptions.mode === 'detect_tables' ? 'active' : ''}`} aria-pressed={activeReview.pdfOptions.mode === 'detect_tables'} onClick={() => updatePdfOptions(activeReview.id, { ...activeReview.pdfOptions, mode: 'detect_tables' })}><ScanSearch size={20} /><span><strong>Find tables</strong><small>Reconstruct bordered tables locally</small></span>{activeReview.pdfOptions.mode === 'detect_tables' && <Check size={17} />}</button>
        </div>
        {activeReview.pdfOptions.mode === 'selected_pages' && <label className="page-range-field"><span>Pages to convert</span><input value={activeReview.pdfOptions.page_selection ?? ''} onChange={event => updatePdfOptions(activeReview.id, { ...activeReview.pdfOptions, page_selection: event.target.value || null })} placeholder="All pages, or 1-3, 7, 10-12" /><small>Leave blank for every page. Page numbers are one-based.</small></label>}
      </section>}
      {activeReview.format === 'xlsx' && <section className="format-protection-options" aria-labelledby="format-protection-heading">
        <h3 id="format-protection-heading">Format Protection</h3>
        <label className="checkbox-label"><input type="checkbox" checked={activeReview.formatProtection} disabled={activeReview.edits.length > 0 || activeReview.sheetNames.some((name, index) => name !== activeReview.preview?.sheets[index]?.name)} onChange={event => setQueue(current => current.map(item => item.id === activeReview.id ? { ...item, formatProtection: event.target.checked, reviewConfirmed: false } : item))} /><span>Keep original styles, merged cells, formulas, links, and embedded content in the exported XLSX.</span></label>
        <p>Available for unchanged XLSX workbooks. Cell corrections or worksheet renaming require a values-only export.</p>
      </section>}
      <div className="preview-toolbar">
        <div>{activeReview.preview ? <><strong>{activeReview.preview.sheets.length} {activeReview.preview.sheets.length === 1 ? 'sheet' : 'sheets'}</strong><span>{activeReview.preview.sheets.reduce((sum, sheet) => sum + sheet.total_rows, 0).toLocaleString()} rows prepared{activeReview.preview.detected_tables ? ` · ${activeReview.preview.detected_tables} table candidates` : ''}</span></> : <><strong>Preview not generated</strong><span>Generate it to inspect and correct the workbook data.</span></>}</div>
        <button className="button button-secondary" disabled={previewLoading} onClick={() => void preparePreview(activeReview)}>{previewLoading ? <LoaderCircle size={16} className="spin" /> : <RefreshCw size={16} />}{activeReview.preview ? 'Refresh preview' : 'Generate preview'}</button>
      </div>
      {previewLoading ? <div className="preview-loading"><LoaderCircle size={26} className="spin" /><strong>{activeReview.pdfOptions.mode === 'detect_tables' ? 'Scanning for tables…' : 'Preparing your preview…'}</strong><span>The source stays on this device.</span></div> : activeReview.preview && <>
        {activeReview.formatProtection ? <div className="preview-warnings"><p><Info size={14} />The grid shows extracted values for review. Export will copy the original XLSX exactly, including its formatting and workbook features.</p></div> : activeReview.preview.warnings.length > 0 && <div className="preview-warnings">{activeReview.preview.warnings.map((warning, index) => <p key={index}><Info size={14} />{warning}</p>)}</div>}
        {activeReview.preview.sheets.length === 0 ? <div className="no-table-results"><Table2 size={34} strokeWidth={1.4} /><h3>No table candidates found</h3><p>Try Select pages to review the relevant text manually, or run OCR on image-only pages before importing them.</p></div> : <>
          <div className="sheet-tabs" role="tablist" aria-label="Preview worksheets">{activeReview.preview.sheets.map((sheet, index) => <button key={`${sheet.name}-${index}`} role="tab" aria-selected={activeSheet === index} className={activeSheet === index ? 'active' : ''} onClick={() => setActiveSheet(index)}>{sheet.name}<span>{sheet.total_rows.toLocaleString()}</span></button>)}</div>
          {activeReview.preview.sheets[activeSheet] && <div className="sheet-review">
            <div className="sheet-review-heading"><label><span>Worksheet name</span><input value={activeReview.sheetNames[activeSheet] ?? activeReview.preview.sheets[activeSheet].name} onChange={event => updateSheetName(activeReview.id, activeSheet, event.target.value)} maxLength={31} /></label><span><CheckCircle2 size={14} />Changes save to this review</span></div>
            <div className="preview-grid-wrap"><table className="preview-grid"><tbody>{activeReview.preview.sheets[activeSheet].rows.map((row, rowIndex) => <tr key={rowIndex}><th scope="row">{rowIndex + 1}</th>{row.map((cell, columnIndex) => <td key={columnIndex}><textarea aria-label={`Row ${rowIndex + 1}, column ${columnIndex + 1}`} rows={Math.min(8, Math.max(1, String(cell ?? '').split('\n').length))} value={cell === null ? '' : String(cell)} onChange={event => updatePreviewCell(activeReview.id, activeSheet, rowIndex, columnIndex, event.target.value)} /></td>)}</tr>)}</tbody></table></div>
            <div className="preview-grid-footer"><span>Showing {activeReview.preview.sheets[activeSheet].rows.length.toLocaleString()} of {activeReview.preview.sheets[activeSheet].total_rows.toLocaleString()} rows{activeReview.preview.sheets[activeSheet].total_columns > 0 ? ` · ${Math.min(activeReview.preview.sheets[activeSheet].total_columns, 50)} of ${activeReview.preview.sheets[activeSheet].total_columns} columns` : ''}</span>{activeReview.preview.sheets[activeSheet].truncated && <span><Info size={13} />Only the preview is limited; export keeps all source rows and columns.</span>}</div>
          </div>}
        </>}
      </>}
      <div className="review-dialog-actions"><p><Eye size={14} />Edit any cell that needs correction, then confirm the review.</p><div><button className="button button-secondary" disabled={previewLoading} onClick={() => setReviewId(null)}>Close</button><button className="button button-primary" disabled={previewLoading || !activeReview.preview || activeReview.preview.sheets.length === 0 || activeReview.sheetNames.some(name => !name.trim())} onClick={() => { setQueue(current => current.map(item => item.id === activeReview.id ? { ...item, reviewConfirmed: true } : item)); setReviewId(null); setNotice(`${activeReview.name} is reviewed and ready to convert.`); }}><Check size={16} />Use this preview</button></div></div>
    </Modal>}
    {termsOpen && <Modal title="Before your first conversion" className="terms-dialog" error={dialogError} onClose={() => { if (!saving) { setTermsOpen(false); setPendingConversion(false); } }}><p className="dialog-intro">Read the terms, then accept them to start using Sheetshift. Version {TERMS_VERSION}.</p><div className="dialog-scroll" tabIndex={0} aria-label="Terms and conditions text"><TermsContent /></div><div className="dialog-actions"><label className="checkbox-label"><input type="checkbox" checked={termsChecked} onChange={event => setTermsChecked(event.target.checked)} disabled={saving} />I have read and accept these terms and conditions.</label><div className="dialog-button-row"><button className="button button-secondary" disabled={saving} onClick={() => { setTermsOpen(false); setPendingConversion(false); }}>Not now</button><button className="button button-primary" disabled={!termsChecked || saving} onClick={() => void acceptTerms()}>{saving && <LoaderCircle size={16} className="spin" />}{pendingConversion ? 'Accept and convert' : 'Accept terms'}</button></div></div></Modal>}
    {clearOpen && <Modal title="Clear conversion history?" error={dialogError} onClose={() => { if (!saving) setClearOpen(false); }}><p className="dialog-intro">This removes all saved conversion records on this device. Your source files and Excel workbooks stay unchanged. This cannot be undone.</p><div className="dialog-button-row"><button className="button button-secondary" disabled={saving} onClick={() => setClearOpen(false)}>Keep history</button><button className="button button-danger" disabled={saving} onClick={() => void clearHistory()}>{saving ? <LoaderCircle size={16} className="spin" /> : <Trash2 size={16} />}Clear history</button></div></Modal>}
  </div>;
}
