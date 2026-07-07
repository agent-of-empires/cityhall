export interface NavItem {
  title: string;
  href: string;
  description?: string;
}

export interface NavSection {
  title: string;
  items: NavItem[];
}

// Single source for the sidebar. Every entry must have a matching page synced
// by scripts/sync-docs.mjs (the sync script fails the build if one is missing).
export const docsNav: NavSection[] = [
  {
    title: "Getting Started",
    items: [
      { title: "Overview", href: "/docs/" },
      { title: "Quick Start", href: "/docs/quick-start/" },
    ],
  },
  {
    title: "Operate",
    items: [
      { title: "Configuration", href: "/docs/configuration/" },
      { title: "Deployment", href: "/docs/deployment/" },
      { title: "CLI Reference", href: "/docs/cli/" },
      { title: "API Reference", href: "/docs/api/" },
    ],
  },
  {
    title: "Contribute",
    items: [
      { title: "Development", href: "/docs/development/" },
    ],
  },
];

export function getFlatNavItems(): NavItem[] {
  return docsNav.flatMap((section) => section.items);
}
