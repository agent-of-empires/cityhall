import clsx from "clsx";
import { Check, ChevronDown, X } from "lucide-react";
import { useEffect } from "react";
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
  SelectHTMLAttributes,
  TextareaHTMLAttributes,
} from "react";

export function Button({
  className,
  variant = "default",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "default" | "primary" | "danger" | "ghost" | "text";
}) {
  return (
    <button
      className={clsx(
        "inline-flex h-8 items-center justify-center gap-1.5 rounded-md border px-3.5 text-[13px] font-medium transition-colors",
        "focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-50",
        {
          "border-border-soft bg-surface-elevated text-text hover:border-border hover:bg-surface":
            variant === "default",
          "border-transparent bg-accent text-white hover:bg-accent-hover": variant === "primary",
          "border-error/30 bg-error/10 text-error hover:bg-error/20": variant === "danger",
          "border-transparent bg-transparent text-text-dim hover:bg-surface-hover hover:text-text": variant === "ghost",
          "h-auto border-none bg-transparent px-1 text-accent hover:text-accent-bright": variant === "text",
        },
        className,
      )}
      {...props}
    />
  );
}

/** Square, borderless button for a lone icon (close, chevron, inline action). */
export function IconButton({ className, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      className={clsx(
        "inline-flex h-7 w-7 items-center justify-center rounded-md text-text-hint transition-colors",
        "hover:bg-surface-hover hover:text-text",
        "focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

const fieldClass =
  "w-full rounded-md border border-border-soft bg-surface-elevated px-2.5 py-[7px] text-[13px] text-text placeholder:text-text-hint focus:border-accent focus:ring-2 focus:ring-accent-soft focus:outline-none disabled:cursor-not-allowed disabled:opacity-50";

export function Input({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={clsx(fieldClass, className)} {...props} />;
}

export function Textarea({ className, ...props }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={clsx(fieldClass, "resize-y", className)} {...props} />;
}

export function Select({ className, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <div className="relative">
      <select className={clsx(fieldClass, "appearance-none pr-8", className)} {...props} />
      <ChevronDown
        size={14}
        className="pointer-events-none absolute top-1/2 right-2.5 -translate-y-1/2 text-text-dim"
      />
    </div>
  );
}

export function Field({ label, help, children }: { label: string; help?: ReactNode; children: ReactNode }) {
  return (
    <label className="block space-y-1.5">
      <SectionLabel>{label}</SectionLabel>
      {children}
      {help && <p className="text-[11.5px] leading-relaxed text-text-hint">{help}</p>}
    </label>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  return <p className="text-sm text-error">{children}</p>;
}

/** All-caps mono eyebrow: field labels, card titles, table headers. */
export function SectionLabel({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <span className={clsx("block font-mono text-[10.5px] tracking-[0.12em] text-text-hint uppercase", className)}>
      {children}
    </span>
  );
}

export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div className={clsx("rounded-card border border-border-soft bg-surface px-[22px] py-[18px]", className)}>
      {children}
    </div>
  );
}

export function Banner({
  className,
  tone = "neutral",
  children,
}: {
  className?: string;
  tone?: "neutral" | "warning" | "error";
  children: ReactNode;
}) {
  return (
    <div
      className={clsx(
        "rounded-lg border px-3.5 py-2.5 text-[13px]",
        {
          "border-border-soft bg-surface-elevated text-text-dim": tone === "neutral",
          "border-waiting/30 bg-waiting/10 text-waiting": tone === "warning",
          "border-error/30 bg-error/10 text-error": tone === "error",
        },
        className,
      )}
    >
      {children}
    </div>
  );
}

export function Tag({ className, accent, children }: { className?: string; accent?: boolean; children: ReactNode }) {
  return (
    <span
      className={clsx(
        "inline-block rounded-[3px] px-1.5 py-px font-mono text-[10px]",
        accent ? "bg-accent-soft text-accent" : "bg-surface-elevated text-text-dim",
        className,
      )}
    >
      {children}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded-[3px] border border-border-soft bg-surface px-1.5 py-px font-mono text-[10.5px] text-text-dim">
      {children}
    </kbd>
  );
}

export type StatusTone = "running" | "waiting" | "idle" | "error";

export const STATUS_TONE_CLASS: Record<StatusTone, string> = {
  running: "text-running",
  waiting: "text-waiting",
  idle: "text-idle",
  error: "text-error",
};

/** Mono status line with a leading glyph, matching the TUI's status vocabulary. */
export function StatusText({
  tone,
  glyph,
  className,
  title,
  children,
}: {
  tone: StatusTone;
  glyph: string;
  className?: string;
  title?: string;
  children: ReactNode;
}) {
  return (
    <span
      title={title}
      className={clsx("inline-flex items-center gap-1.5 font-mono text-[12.5px]", STATUS_TONE_CLASS[tone], className)}
    >
      <span aria-hidden>{glyph}</span>
      {children}
    </span>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={clsx(
        "relative h-[22px] w-[38px] shrink-0 rounded-full transition-colors",
        "focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:outline-none",
        "disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-accent" : "bg-border",
      )}
    >
      <span
        className={clsx(
          "absolute top-0.5 h-[18px] w-[18px] rounded-full bg-text-bright transition-[left]",
          checked ? "left-[18px]" : "left-0.5",
        )}
      />
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  className,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (next: T) => void;
  className?: string;
}) {
  return (
    <div
      className={clsx("inline-flex gap-0.5 rounded-lg border border-border-soft bg-surface-elevated p-0.5", className)}
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
          className={clsx(
            "rounded-md px-3 py-[5px] text-[13px] transition-colors",
            "focus-visible:ring-2 focus-visible:ring-accent/40 focus-visible:outline-none",
            value === option.value ? "bg-accent text-white" : "text-text-dim hover:text-text",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function Checkbox({
  checked,
  onChange,
  label,
  disabled,
  className,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: ReactNode;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <label className={clsx("inline-flex cursor-pointer items-center gap-2 text-[13px] text-text", className)}>
      <input
        type="checkbox"
        className="peer sr-only"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span
        className={clsx(
          "inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-[3px] border-[1.5px] text-accent transition-colors",
          "peer-focus-visible:ring-2 peer-focus-visible:ring-accent/40",
          checked ? "border-accent" : "border-border",
        )}
      >
        {checked && <Check size={11} strokeWidth={3} />}
      </span>
      {label}
    </label>
  );
}

/** Thin horizontal usage bar. `percent` is clamped, so callers can pass raw ratios. */
export function Meter({ percent, danger }: { percent: number; danger?: boolean }) {
  const clamped = Math.max(0, Math.min(100, percent));
  return (
    <div className="h-1.5 overflow-hidden rounded-full bg-surface-elevated">
      <div
        className={clsx("h-full rounded-full", danger && clamped > 90 ? "bg-error" : "bg-accent")}
        style={{ width: `${clamped}%` }}
      />
    </div>
  );
}

export function Modal({
  title,
  onClose,
  footer,
  className,
  children,
}: {
  title: ReactNode;
  onClose?: () => void;
  footer?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  // Backdrop click and Escape both dismiss, so a dialog is never a trap.
  useEffect(() => {
    if (!onClose) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="animate-fade-in fixed inset-0 z-50 flex items-center justify-center bg-[rgba(2,6,23,0.65)] p-6"
      onClick={onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={typeof title === "string" ? title : undefined}
        onClick={(e) => e.stopPropagation()}
        className={clsx(
          "animate-slide-up flex max-h-[calc(100dvh-48px)] w-full max-w-[520px] flex-col overflow-hidden",
          "rounded-xl border border-border-soft bg-surface shadow-[0_24px_64px_rgba(0,0,0,0.5)]",
          className,
        )}
      >
        <div className="flex items-center justify-between border-b border-border-soft px-[18px] py-3.5">
          <span className="font-mono text-xs text-text-dim">{title}</span>
          {onClose && (
            <IconButton onClick={onClose} aria-label="Close">
              <X size={15} />
            </IconButton>
          )}
        </div>
        <div className="flex-1 overflow-y-auto px-[22px] py-[18px]">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2.5 border-t border-border-soft px-[18px] py-3.5">
            {footer}
          </div>
        )}
      </div>
    </div>
  );
}

/* Tables are plain markup plus these classes: one card shell, one header row,
   one body row. Cheaper than a component per cell, and pages keep their own
   column layout. */
export const tableCardClass = "overflow-hidden rounded-card border border-border-soft bg-surface";
export const tableHeadClass =
  "border-b border-border-soft text-left font-mono text-[10.5px] uppercase tracking-[0.12em] text-text-hint";
export const thClass = "px-4 py-2.5 font-normal";
export const trClass = "border-b border-border-soft/60 transition-colors last:border-0 hover:bg-surface-hover";
export const tdClass = "px-4 py-3";
