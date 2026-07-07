import { LogOut } from "lucide-react";
import { api, type Me } from "../lib/api";
import { Button } from "./ui";

export function TopBar({ me, onLogout }: { me: Me; onLogout: () => Promise<void> }) {
  async function logout() {
    await api.logout();
    await onLogout();
  }

  return (
    <header className="flex h-12 items-center justify-between border-b border-surface-700 px-4">
      <span className="font-mono text-sm font-medium tracking-wider text-text-bright">CityHall</span>
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
