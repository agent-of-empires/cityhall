/** Settings is one page per tab, so the sidebar sub-nav and the router agree on
    the same list of slugs. */
export type SettingsTab = {
  slug: string;
  label: string;
};

export const SETTINGS_TABS: SettingsTab[] = [
  { slug: "access", label: "Access & sign-in" },
  { slug: "signup", label: "Sign-up" },
  { slug: "email", label: "Email" },
  { slug: "workspace-defaults", label: "Workspace defaults" },
  { slug: "workspace-config", label: "Workspace config" },
  { slug: "advanced", label: "Advanced" },
];

export const DEFAULT_SETTINGS_TAB = SETTINGS_TABS[0].slug;

export function settingsTabLabel(slug: string | undefined): string {
  return SETTINGS_TABS.find((tab) => tab.slug === slug)?.label ?? "Settings";
}
