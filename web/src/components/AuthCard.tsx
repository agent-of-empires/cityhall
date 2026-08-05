import type { ReactNode } from "react";

/**
 * Full-viewport shell shared by the six auth screens: soft glow backdrop,
 * fade-in entrance, logo wordmark, and a fixed-width column for the page's
 * own heading and form.
 */
export function AuthCard({ children }: { children: ReactNode }) {
  return (
    <div className="relative flex h-screen items-center justify-center overflow-hidden bg-canvas p-4">
      <div className="pointer-events-none absolute -top-[260px] left-1/2 h-[520px] w-[900px] -translate-x-1/2 bg-[radial-gradient(ellipse_at_center,rgba(217,119,6,0.16),transparent_62%)]" />
      <div className="pointer-events-none absolute -top-[200px] left-[32%] h-[420px] w-[620px] bg-[radial-gradient(ellipse_at_center,rgba(13,148,136,0.10),transparent_65%)]" />
      <div className="animate-fade-in relative flex w-[var(--width-dialog)] flex-col gap-[22px]">
        <div className="flex items-center gap-2.5">
          <img src="/logo.svg" alt="" className="h-[21px] w-[21px]" />
          <span className="font-mono text-base font-semibold text-text-bright">CityHall</span>
        </div>
        {children}
      </div>
    </div>
  );
}
