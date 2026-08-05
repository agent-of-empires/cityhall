import clsx from "clsx";
import { Boxes, ExternalLink, LayoutDashboard, LogOut, Settings, Shield, Users } from "lucide-react";
import { useEffect, useState, type ComponentType, type ReactNode } from "react";
import { NavLink, Outlet, useLocation } from "react-router-dom";
import { api, can, type Me } from "../lib/api";
import { SETTINGS_TABS } from "../lib/settingsTabs";
import { IconButton } from "./ui";

type NavItem = {
  to: string;
  label: string;
  icon: ComponentType<{ size?: number }>;
  /** Right-aligned count, shown once the sidebar knows it. */
  badge?: number;
  /** Sub-nav rendered while this item is the active one. */
  children?: { to: string; label: string }[];
};

type NavGroup = { label?: string; items: NavItem[] };

function useNavGroups(me: Me): NavGroup[] {
  const [counts, setCounts] = useState<{ users?: number; roles?: number }>({});

  // Counts are decoration, so a failed fetch just leaves the badge off.
  useEffect(() => {
    if (can(me, "users.read")) {
      api
        .listUsers()
        .then((users) => setCounts((c) => ({ ...c, users: users.length })))
        .catch(() => {});
    }
    if (can(me, "roles.read")) {
      api
        .listRoles()
        .then((roles) => setCounts((c) => ({ ...c, roles: roles.length })))
        .catch(() => {});
    }
  }, [me]);

  const groups: NavGroup[] = [];

  if (can(me, "dashboard.read")) {
    groups.push({ label: "Overview", items: [{ to: "/dashboard", label: "Dashboard", icon: LayoutDashboard }] });
  }

  const people: NavItem[] = [];
  if (can(me, "users.read")) {
    people.push({ to: "/users", label: "Users", icon: Users, badge: counts.users });
  }
  if (can(me, "roles.read")) {
    people.push({ to: "/roles", label: "Roles", icon: Shield, badge: counts.roles });
  }
  if (people.length) groups.push({ label: "People", items: people });

  if (can(me, "workspaces.read")) {
    groups.push({ label: "Compute", items: [{ to: "/workspaces", label: "Workspaces", icon: Boxes }] });
  } else if (can(me, "workspaces.use")) {
    // A member has one workspace, so the nav names it rather than the fleet.
    groups.push({ items: [{ to: "/workspaces", label: "My workspace", icon: Boxes }] });
  }

  if (can(me, "settings.read")) {
    groups.push({
      label: "Configure",
      items: [
        {
          to: "/settings",
          label: "Settings",
          icon: Settings,
          children: SETTINGS_TABS.map((tab) => ({ to: `/settings/${tab.slug}`, label: tab.label })),
        },
      ],
    });
  }

  return groups;
}

function NavRow({ item, active }: { item: NavItem; active: boolean }) {
  const Icon = item.icon;
  return (
    <NavLink
      to={item.to}
      className={clsx(
        "flex items-center gap-2.5 border-l-2 px-[18px] py-[7px] text-[13.5px] transition-colors",
        active
          ? "border-accent bg-surface-active font-medium text-text-bright"
          : "border-transparent text-text-dim hover:bg-surface-hover hover:text-text",
      )}
    >
      <Icon size={15} />
      <span className="flex-1">{item.label}</span>
      {item.badge !== undefined && <span className="font-mono text-[11px] text-text-faint">{item.badge}</span>}
    </NavLink>
  );
}

export function AppShell({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  const groups = useNavGroups(me);
  const { pathname } = useLocation();
  const [workspaceOrigin, setWorkspaceOrigin] = useState<string | null>(null);

  useEffect(() => {
    if (!can(me, "workspaces.use")) return;
    api
      .myWorkspace()
      .then((w) => setWorkspaceOrigin(w.proxy_origin))
      .catch(() => {});
  }, [me]);

  async function logout() {
    await api.logout();
    await onLogout();
  }

  return (
    <div className="flex h-full min-h-0">
      <aside className="flex w-[244px] shrink-0 flex-col border-r border-border-soft bg-surface">
        <div className="flex h-14 items-center gap-2.5 border-b border-border-soft px-[18px]">
          <img src="/logo.svg" alt="" className="h-[21px] w-[21px]" />
          <span className="font-mono text-sm font-semibold text-text-bright">CityHall</span>
        </div>

        <nav className="flex-1 overflow-y-auto py-3">
          {groups.map((group, i) => (
            <div key={group.label ?? i}>
              {group.label && (
                <div
                  className={clsx(
                    "px-[18px] pb-[5px] font-mono text-[10px] tracking-[0.14em] text-text-faint uppercase",
                    i === 0 ? "pt-2.5" : "pt-3.5",
                  )}
                >
                  {group.label}
                </div>
              )}
              {group.items.map((item) => {
                const active = pathname === item.to || pathname.startsWith(`${item.to}/`);
                return (
                  <div key={item.to}>
                    <NavRow item={item} active={active} />
                    {active &&
                      item.children?.map((child) => (
                        <NavLink
                          key={child.to}
                          to={child.to}
                          className={({ isActive }) =>
                            clsx(
                              "block border-l-2 py-[5px] pr-[18px] pl-[45px] text-[12.5px] transition-colors",
                              isActive
                                ? "border-accent bg-surface-active text-text-bright"
                                : "border-transparent text-text-dim hover:text-text",
                            )
                          }
                        >
                          {child.label}
                        </NavLink>
                      ))}
                  </div>
                );
              })}
            </div>
          ))}
        </nav>

        <div className="border-t border-border-soft px-3 py-2.5">
          {workspaceOrigin && (
            <a
              // The exit param ends any admin access to another user's
              // workspace first, so this link always opens YOUR workspace.
              href={`${workspaceOrigin}/?cityhall_ws_exit=1`}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-2.5 rounded-md px-1.5 py-1.5 text-[13px] text-text-dim transition-colors hover:bg-surface-hover hover:text-text"
            >
              <ExternalLink size={15} />
              Open workspace
            </a>
          )}
          <div className="flex items-center gap-1">
            <NavLink
              to="/account"
              className={({ isActive }) =>
                clsx(
                  "flex min-w-0 flex-1 items-center gap-2.5 rounded-md px-1.5 py-1.5 text-[13px] transition-colors hover:bg-surface-hover",
                  isActive ? "text-text-bright" : "text-text-dim hover:text-text",
                )
              }
            >
              <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-accent-soft font-mono text-[10px] font-medium text-accent uppercase">
                {me.username.slice(0, 1)}
              </span>
              <span className="truncate">{me.username}</span>
            </NavLink>
            <IconButton onClick={logout} aria-label="Log out" title="Log out">
              <LogOut size={15} />
            </IconButton>
          </div>
        </div>
      </aside>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <Outlet />
      </div>
    </div>
  );
}

/** The 56px header inside the content column: page title, meta, page actions. */
export function PageHeader({ title, meta, actions }: { title: ReactNode; meta?: ReactNode; actions?: ReactNode }) {
  return (
    <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border-soft px-[26px]">
      <div className="flex min-w-0 items-baseline gap-2.5">
        <h1 className="truncate text-[17px] font-semibold tracking-[-0.01em] text-text-bright">{title}</h1>
        {meta && <span className="truncate font-mono text-[11.5px] text-text-faint">{meta}</span>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}

/** Scrolling content area under the page header. */
export function PageBody({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={clsx("flex-1 overflow-y-auto px-[26px] pt-[22px] pb-12", className)}>{children}</div>;
}
