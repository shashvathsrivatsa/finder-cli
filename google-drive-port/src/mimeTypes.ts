const SOURCE_MIME: Record<string, string> = {
  ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  ".doc":  "application/msword",
  ".txt":  "text/plain",
  ".rtf":  "application/rtf",
  ".odt":  "application/vnd.oasis.opendocument.text",
  ".html": "text/html",
  ".htm":  "text/html",
  ".csv":  "text/csv",
  ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  ".xls":  "application/vnd.ms-excel",
  ".ods":  "application/vnd.oasis.opendocument.spreadsheet",
  ".tsv":  "text/tab-separated-values",
};

const DOCS_EXTS  = new Set([".docx", ".doc", ".txt", ".rtf", ".odt", ".html", ".htm"]);
const SHEETS_EXTS = new Set([".csv", ".xlsx", ".xls", ".ods", ".tsv"]);

const GOOGLE_DOCS_MIME   = "application/vnd.google-apps.document";
const GOOGLE_SHEETS_MIME = "application/vnd.google-apps.spreadsheet";

export function getMimeInfo(ext: string): { sourceMime: string; targetMime: string; appType: "docs" | "sheets" } {
  const sourceMime = SOURCE_MIME[ext];
  if (!sourceMime) throw new Error(`Unsupported file extension: ${ext}`);

  if (DOCS_EXTS.has(ext))   return { sourceMime, targetMime: GOOGLE_DOCS_MIME,   appType: "docs" };
  if (SHEETS_EXTS.has(ext)) return { sourceMime, targetMime: GOOGLE_SHEETS_MIME, appType: "sheets" };

  throw new Error(`No Google target for extension: ${ext}`);
}

export function getFileUrl(fileId: string, appType: "docs" | "sheets"): string {
  return appType === "docs"
    ? `https://docs.google.com/document/d/${fileId}/edit`
    : `https://docs.google.com/spreadsheets/d/${fileId}/edit`;
}
