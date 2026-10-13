import { useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

const focusableSelector =
  'a[href], button, input, select, textarea, [tabindex], [contenteditable="true"]';
const modalStack: HTMLElement[] = [];
const isolatedElements = new WeakMap<
  HTMLElement,
  { count: number; inert: boolean; ariaHidden: string | null }
>();
let previousBodyOverflow = "";

function focusableElements(modal: HTMLElement): HTMLElement[] {
  return Array.from(modal.querySelectorAll<HTMLElement>(focusableSelector)).filter(
    (element) =>
      element.tabIndex >= 0 &&
      !element.matches(":disabled") &&
      !element.closest("[hidden], [inert]") &&
      getComputedStyle(element).display !== "none" &&
      getComputedStyle(element).visibility !== "hidden",
  );
}

function isolate(element: HTMLElement) {
  const existing = isolatedElements.get(element);
  if (existing) {
    existing.count += 1;
    return;
  }
  isolatedElements.set(element, {
    count: 1,
    inert: element.hasAttribute("inert"),
    ariaHidden: element.getAttribute("aria-hidden"),
  });
  element.setAttribute("inert", "");
  element.setAttribute("aria-hidden", "true");
}

function restore(element: HTMLElement) {
  const previous = isolatedElements.get(element);
  if (!previous || --previous.count > 0) return;
  if (!previous.inert) element.removeAttribute("inert");
  if (previous.ariaHidden === null) element.removeAttribute("aria-hidden");
  else element.setAttribute("aria-hidden", previous.ariaHidden);
  isolatedElements.delete(element);
}

type ModalProps = {
  children: ReactNode;
  labelledBy: string;
  className: string;
  onDismiss: () => void;
  dismissible?: boolean;
  closeOnBackdrop?: boolean;
  initialFocus?: string;
};

export function Modal({
  children,
  labelledBy,
  className,
  onDismiss,
  dismissible = true,
  closeOnBackdrop = false,
  initialFocus,
}: ModalProps) {
  const modalRef = useRef<HTMLDivElement>(null);
  const interactionRef = useRef({ onDismiss, dismissible });
  useLayoutEffect(() => {
    interactionRef.current = { onDismiss, dismissible };
  }, [onDismiss, dismissible]);

  useLayoutEffect(() => {
    const modal = modalRef.current;
    if (!modal) return;
    const opener = document.activeElement;
    const background = Array.from(document.body.children).filter(
      (element): element is HTMLElement =>
        element instanceof HTMLElement && element !== modal,
    );
    const focusFirst = () => {
      const preferred = initialFocus
        ? modal.querySelector<HTMLElement>(initialFocus)
        : null;
      (preferred ?? focusableElements(modal)[0] ?? modal).focus();
    };
    if (modalStack.length === 0) {
      previousBodyOverflow = document.body.style.overflow;
      document.body.style.overflow = "hidden";
    }
    modalStack.push(modal);
    // Move focus before hiding its former ancestor from assistive technology.
    focusFirst();
    background.forEach(isolate);

    const onKeyDown = (event: KeyboardEvent) => {
      if (modalStack[modalStack.length - 1] !== modal) return;
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        if (interactionRef.current.dismissible) interactionRef.current.onDismiss();
      }
      if (event.key !== "Tab") return;
      const controls = focusableElements(modal);
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (!first || !last) {
        event.preventDefault();
        modal.focus();
      } else if (!modal.contains(document.activeElement)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (
        event.shiftKey &&
        (document.activeElement === first || document.activeElement === modal)
      ) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    const onFocusIn = (event: FocusEvent) => {
      if (
        modalStack[modalStack.length - 1] === modal &&
        !modal.contains(event.target as Node)
      ) {
        focusFirst();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("focusin", onFocusIn);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("focusin", onFocusIn);
      modalStack.splice(modalStack.indexOf(modal), 1);
      background.forEach(restore);
      if (modalStack.length === 0) document.body.style.overflow = previousBodyOverflow;
      if (
        opener instanceof HTMLElement &&
        opener.isConnected &&
        !opener.closest("[inert]")
      ) {
        opener.focus();
      } else {
        const remainingModal = modalStack[modalStack.length - 1];
        if (remainingModal)
          (focusableElements(remainingModal)[0] ?? remainingModal).focus();
      }
    };
  }, [initialFocus]);

  return createPortal(
    <div
      ref={modalRef}
      aria-labelledby={labelledBy}
      aria-modal="true"
      className={`ingest-modal text-slate-950 dark:text-slate-50 ${className}`}
      onClick={(event) => {
        if (closeOnBackdrop && dismissible && event.target === event.currentTarget) {
          onDismiss();
        }
      }}
      role="dialog"
      tabIndex={-1}
    >
      {children}
    </div>,
    document.body,
  );
}
