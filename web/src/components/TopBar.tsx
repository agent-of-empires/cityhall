import clsx from "clsx";
import { LogOut, Settings, Shield, Users } from "lucide-react";
import { NavLink } from "react-router-dom";
import { api, can, type Me } from "../lib/api";
import { Button } from "./ui";

const navLinkClass = ({ isActive }: { isActive: boolean }) =>
  clsx(
    "flex items-center gap-1.5 rounded-md px-2 py-1 text-sm transition-colors",
    isActive ? "text-text-primary" : "text-text-secondary hover:text-text-primary",
  );

export function TopBar({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  async function logout() {
    await api.logout();
    await onLogout();
  }

  return (
    <header className="flex h-12 items-center justify-between border-b border-surface-700 px-4">
      <div className="flex items-center gap-4">
        <span className="font-mono text-sm font-medium tracking-wider text-text-bright">CityHall</span>
        <nav className="flex items-center gap-1">
          <NavLink to="/" end className={navLinkClass}>
            <Users size={14} />
            Users
          </NavLink>
          {can(me, "roles.read") && (
            <NavLink to="/roles" className={navLinkClass}>
              <Shield size={14} />
              Roles
            </NavLink>
          )}
          {can(me, "settings.read") && (
            <NavLink to="/settings" className={navLinkClass}>
              <Settings size={14} />
              Settings
            </NavLink>
          )}
        </nav>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-sm text-text-secondary">{me.username}</span>
        <Button variant="ghost" onClick={logout} className="flex items-center gap-1.5">
          <LogOut size={14} />
          Logout
        </Button>
      </div>
    </header>
  );
}
