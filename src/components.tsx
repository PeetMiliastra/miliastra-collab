import { useEffect, useRef } from "react";
import { LoaderCircle, X } from "lucide-react";
export function Modal({
  title,
  subtitle,
  children,
  onClose,
  closeDisabled = false,
  className,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
  onClose: () => void;
  closeDisabled?: boolean;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef({ onClose, closeDisabled });
  closeRef.current = { onClose, closeDisabled };
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !closeRef.current.closeDisabled)
        closeRef.current.onClose();
      if (e.key === "Tab") {
        const nodes = ref.current?.querySelectorAll<HTMLElement>(
          "button:not(:disabled),input:not(:disabled),textarea:not(:disabled),select:not(:disabled),summary",
        );
        if (!nodes?.length) {
          e.preventDefault();
          ref.current?.focus();
          return;
        }
        const first = nodes[0],
          last = nodes[nodes.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("keydown", key);
      previous?.focus();
    };
  }, []);
  return (
    <div className="modal-backdrop">
      <div
        className={`modal${className ? ` ${className}` : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        ref={ref}
      >
        <button
          className="close-modal"
          aria-label="Close"
          disabled={closeDisabled}
          onClick={onClose}
        >
          <X size={20} />
        </button>
        <h2>{title}</h2>
        {subtitle && <p className="modal-subtitle">{subtitle}</p>}
        {children}
      </div>
    </div>
  );
}

/** The spinner and label shown while something is running. */
export function Progress({ label }: { label: string }) {
  return (
    <div className="progress" role="status">
      <LoaderCircle size={16} className="spin" />
      <span>{label}</span>
    </div>
  );
}

/** A yes-or-no question, in place of the browser's confirm box. */
export function Confirm({
  title,
  children,
  confirmLabel,
  danger = false,
  busy = false,
  onConfirm,
  onClose,
}: {
  title: string;
  children: React.ReactNode;
  confirmLabel: string;
  danger?: boolean;
  busy?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Modal title={title} onClose={onClose} closeDisabled={busy}>
      {children}
      <div className="modal-actions">
        <button className="button" disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className={`button ${danger ? "danger" : "primary"}`}
          disabled={busy}
          onClick={onConfirm}
        >
          {confirmLabel}
        </button>
      </div>
    </Modal>
  );
}
