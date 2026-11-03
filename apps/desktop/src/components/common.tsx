import {
  Archive,
  File,
  FileText,
  Film,
  Image as ImageIcon,
  Music,
  Package,
  type LucideIcon,
} from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import type { Category, DownloadStatus } from "../api";
import { STATUS_LABEL } from "../format";

export const CATEGORY_ICON: Record<Category, LucideIcon> = {
  video: Film,
  music: Music,
  document: FileText,
  image: ImageIcon,
  archive: Archive,
  software: Package,
  other: File,
};

export function CategoryIcon({ category, size = 18 }: { category: Category; size?: number }) {
  const Icon = CATEGORY_ICON[category];
  return (
    <span className={`cat-icon cat-${category}`} aria-hidden>
      <Icon size={size} strokeWidth={1.75} />
    </span>
  );
}

export function StatusBadge({ status }: { status: DownloadStatus }) {
  return <span className={`status status-${status}`}>{STATUS_LABEL[status]}</span>;
}

/** Barre de progression ; indéterminée si la taille est inconnue. */
export function Progress({ value, status }: { value: number | null; status: DownloadStatus }) {
  const pct = value == null ? null : Math.max(0, Math.min(1, value)) * 100;
  return (
    <div
      className={`progress progress-${status} ${pct == null ? "indeterminate" : ""}`}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct == null ? undefined : Math.round(pct)}
    >
      <div className="progress-fill" style={pct == null ? undefined : { width: `${pct}%` }} />
    </div>
  );
}

// --- Fenêtres modales -------------------------------------------------------

export function Modal({
  title,
  onClose,
  children,
  footer,
  wide,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer: ReactNode;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    ref.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${wide ? "modal-wide" : ""}`} role="dialog" aria-modal aria-label={title} ref={ref}>
        <h2 className="modal-title">{title}</h2>
        <div className="modal-body">{children}</div>
        <div className="modal-footer">{footer}</div>
      </div>
    </div>
  );
}

// --- Notifications internes ------------------------------------------------

interface Toast {
  id: number;
  text: string;
  kind: "info" | "error";
}

const ToastCtx = createContext<(text: string, kind?: Toast["kind"]) => void>(() => {});

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const push = useCallback((text: string, kind: Toast["kind"] = "info") => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, kind }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), kind === "error" ? 7000 : 3500);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.kind}`}>
            {t.text}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

export const useToast = () => useContext(ToastCtx);
