import { useEffect, useRef, type ReactNode } from 'react';

const FOCUSABLE = 'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]';

/** Modal dialog: focus moves in, Tab is trapped, Escape closes, focus returns to the opener. */
export function ImageModal({ title, className = '', onClose, children }: { title: string; className?: string; onClose: () => void; children: ReactNode }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const id = `image-modal-${title.toLowerCase().replace(/[^a-z0-9]+/g, '-')}`;
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null, el = dialog.current;
    if (el?.showModal) el.showModal(); else el?.setAttribute('open', '');
    (el?.querySelector<HTMLElement>('[data-autofocus]') ?? el?.querySelector<HTMLElement>(FOCUSABLE))?.focus();
    return () => { el?.close?.(); opener?.focus(); };
  }, []);
  return <dialog ref={dialog} className={`modal image-modal ${className}`} aria-labelledby={id} aria-modal="true"
    onCancel={e => { e.preventDefault(); onClose(); }}
    onKeyDown={e => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onClose(); }
      if (e.key === 'Tab') {
        const controls = e.currentTarget.querySelectorAll<HTMLElement>(FOCUSABLE);
        const first = controls[0], last = controls[controls.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
      }
    }}>
    <div className="image-modal-heading"><h2 id={id}>{title}</h2><button onClick={onClose}>Close</button></div>
    {children}
  </dialog>;
}
