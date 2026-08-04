import { useCallback, useEffect, useState } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import { api, ApiError, can, type Me } from "./lib/api";
import { DashboardPage } from "./components/DashboardPage";
import { LoginPage } from "./components/LoginPage";
import { ChangePasswordPage } from "./components/ChangePasswordPage";
import { UsersPage } from "./components/UsersPage";
import { RolesPage } from "./components/RolesPage";
import { SettingsPage } from "./components/SettingsPage";
import { AccountPage } from "./components/AccountPage";
import { ForgotPasswordPage } from "./components/ForgotPasswordPage";
import { ResetPasswordPage } from "./components/ResetPasswordPage";
import { RegisterPage } from "./components/RegisterPage";
import { VerifyEmailPage } from "./components/VerifyEmailPage";
import { WorkspacesPage } from "./components/WorkspacesPage";

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      setMe(await api.me());
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) setMe(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (loading) {
    return <div className="flex h-full items-center justify-center text-text-muted">Loading...</div>;
  }

  return (
    <Routes>
      <Route
        path="/login"
        element={
          me ? (
            <Navigate to={me.must_change_password ? "/change-password" : "/"} replace />
          ) : (
            <LoginPage onAuthed={refresh} />
          )
        }
      />
      <Route
        path="/change-password"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : !me.must_change_password ? (
            <Navigate to="/" replace />
          ) : (
            <ChangePasswordPage forced={me.must_change_password} onDone={refresh} />
          )
        }
      />
      {/* `/` redirects rather than rendering a different component per role, so
          every page keeps one stable URL that bookmarks and history can hold. */}
      <Route
        path="/"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : (
            <Navigate to={can(me, "dashboard.read") ? "/dashboard" : "/users"} replace />
          )
        }
      />
      <Route
        path="/dashboard"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : !can(me, "dashboard.read") ? (
            <Navigate to="/users" replace />
          ) : (
            <DashboardPage me={me} onLogout={refresh} />
          )
        }
      />
      <Route
        path="/users"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : (
            <UsersPage me={me} onLogout={refresh} />
          )
        }
      />
      <Route
        path="/roles"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : (
            <RolesPage me={me} onLogout={refresh} />
          )
        }
      />
      <Route
        path="/workspaces"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : (
            <WorkspacesPage me={me} onLogout={refresh} />
          )
        }
      />
      <Route
        path="/account"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : (
            <AccountPage me={me} onLogout={refresh} />
          )
        }
      />
      <Route
        path="/settings"
        element={
          !me ? (
            <Navigate to="/login" replace />
          ) : me.must_change_password ? (
            <Navigate to="/change-password" replace />
          ) : (
            <SettingsPage me={me} onLogout={refresh} />
          )
        }
      />
      <Route path="/forgot-password" element={<ForgotPasswordPage />} />
      <Route path="/reset-password" element={<ResetPasswordPage />} />
      <Route path="/register" element={<RegisterPage />} />
      <Route path="/verify-email" element={<VerifyEmailPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}
