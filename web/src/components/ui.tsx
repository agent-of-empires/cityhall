import clsx from "clsx";
import type { ButtonHTMLAttributes, InputHTMLAttributes, SelectHTMLAttributes } from "react";

export function Button({
  className,
  variant = "default",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "default" | "primary" | "danger" | "ghost";
}) {
  return (
    <button
      className={clsx(
        "rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500",
        "disabled:cursor-not-allowed disabled:opacity-50",
        {
          "bg-surface-800 text-text-primary hover:bg-surface-700": variant === "default",
          "bg-brand-500 text-text-on-brand hover:bg-brand-600": variant === "primary",
          "bg-transparent text-status-error hover:bg-surface-800": variant === "danger",
          "bg-transparent text-text-secondary hover:bg-surface-800 hover:text-text-primary": variant === "ghost",
        },
        className,
      )}
      {...props}
    />
  );
}

export function Input({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={clsx(
        "w-full rounded-md border border-surface-700 bg-surface-950 px-3 py-2 text-sm",
        "text-text-primary placeholder:text-text-muted",
        "focus:border-brand-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500",
        className,
      )}
      {...props}
    />
  );
}

export function Select({ className, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      className={clsx(
        "w-full rounded-md border border-surface-700 bg-surface-950 px-3 py-2 text-sm",
        "text-text-primary",
        "focus:border-brand-500 focus:outline-none focus-visible:ring-2 focus-visible:ring-brand-500",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block space-y-1.5">
      <span className="font-mono text-xs uppercase tracking-wider text-text-muted">{label}</span>
      {children}
    </label>
  );
}

export function ErrorText({ children }: { children: React.ReactNode }) {
  return <p className="text-sm text-status-error">{children}</p>;
}
