import clsx from "clsx";
import { Check, Circle } from "lucide-react";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { api, can, type Me } from "../../lib/api";
import { fetchSetupInputs } from "../../lib/setupInputs";
import { setupDoneCount, setupSteps, type SetupStepState } from "../../lib/setupProgress";
import { Meter } from "../ui";

/** Setup-progress banner: the same seven steps the wizard walks through, read
 *  live from settings so a step already configured outside the wizard (or
 *  dismissed from here) drops off on its own. Hidden once every step is done,
 *  so a finished deployment never sees it again. */
export function SetupChecklist({ me }: { me: Me }) {
  const [steps, setSteps] = useState<SetupStepState[] | null>(null);
  const allowed = can(me, "settings.read");

  useEffect(() => {
    if (!allowed) return;
    let cancelled = false;
    Promise.all([
      fetchSetupInputs(me.must_change_password),
      // A setup-state read that fails must not hide the banner, since that is
      // the one thing it exists to surface.
      api.getSetupState().catch(() => ({ dismissed_steps: [], wizard_finished: false })),
    ]).then(([inputs, state]) => {
      if (!cancelled) setSteps(setupSteps(inputs, state.dismissed_steps));
    });
    return () => {
      cancelled = true;
    };
  }, [allowed, me.must_change_password]);

  if (!allowed || !steps) return null;
  const doneCount = setupDoneCount(steps);
  if (doneCount >= steps.length) return null;

  return (
    <div className="overflow-hidden rounded-card border border-accent/30 bg-surface">
      <div className="h-[2px] bg-gradient-to-r from-transparent via-accent to-transparent" />
      <div className="flex flex-wrap items-center justify-between gap-4 border-b border-border-soft px-[18px] py-[15px]">
        <div>
          <p className="text-[14.5px] font-semibold text-text-bright">Finish setting up CityHall</p>
          <p className="mt-0.5 text-[12.5px] text-text-dim">
            {doneCount} of {steps.length} done. Nothing here blocks your team.
          </p>
        </div>
        <div className="flex items-center gap-3">
          <div className="w-[132px]">
            <Meter percent={(doneCount / steps.length) * 100} />
          </div>
          <Link
            to="/setup"
            className="inline-flex h-8 items-center justify-center rounded-md border border-border-soft bg-surface-elevated px-3.5 text-[13px] font-medium text-text transition-colors hover:border-border hover:bg-surface"
          >
            Resume setup
          </Link>
        </div>
      </div>
      <div className="grid grid-cols-2">
        {steps.map((step) => (
          <SetupRow key={step.key} step={step} />
        ))}
      </div>
    </div>
  );
}

function SetupRow({ step }: { step: SetupStepState }) {
  const rowClass = "flex items-center gap-2.5 border-b border-border-soft px-[18px] py-2.5 transition-colors";
  const content = (
    <>
      {step.done ? (
        <Check size={13} strokeWidth={2.5} className="shrink-0 text-running" />
      ) : (
        <Circle size={9} strokeWidth={2.5} className="shrink-0 text-accent" />
      )}
      <span className={clsx("text-[13px]", step.done ? "text-text-dim" : "text-text")}>{step.label}</span>
      {!step.done && <span className="ml-auto font-mono text-[11.5px] text-link">set up →</span>}
    </>
  );
  if (step.done) return <div className={rowClass}>{content}</div>;
  return (
    <Link to={step.route} className={clsx(rowClass, "hover:bg-surface-hover")}>
      {content}
    </Link>
  );
}
