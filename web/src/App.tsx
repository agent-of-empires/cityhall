import { useCallback, useEffect, useState } from "react";
import { Navigate, Route, Routes } from "react-router-dom";
import { api, ApiError, can, type Me } from "./lib/api";
import { AppShell } from "./components/AppShell";
import { DashboardPage } from "./components/DashboardPage";
import { LoginPage } from "./components/LoginPage";
import { ChangePasswordPage } from "./components/ChangePasswordPage";
import { UsersPage } from "./components/UsersPage";
import { RolesPage } from "./components/RolesPage";
import { SettingsPage } from "./components/SettingsPage";
import { SetupWizardPage } from "./components/SetupWizardPage";
import { AccountPage } from "./components/AccountPage";
import { ForgotPasswordPage } from "./components/ForgotPasswordPage";
import { ResetPasswordPage } from "./components/ResetPasswordPage";
import { RegisterPage } from "./components/RegisterPage";
import { VerifyEmailPage } from "./components/VerifyEmailPage";
import { WorkspacesPage } from "./components/WorkspacesPage";
import { DEFAULT_SETTINGS_TAB } from "./lib/settingsTabs";

/** The first page this user can actually see. Roles are fully custom, so no
 *  permission implies another: a role with only `roles.read` would land on a
 *  blank page if the fallback assumed a workspace. Account needs no permission,
 *  so it is the floor. */
function landingRoute(me: Me): string {
  if (can(me, "dashboard.read")) return "/dashboard";
  if (can(me, "workspaces.read") || can(me, "workspaces.use")) return "/workspaces";
  if (can(me, "users.read")) return "/users";
  if (can(me, "roles.read")) return "/roles";
  if (can(me, "settings.read")) return `/settings/${DEFAULT_SETTINGS_TAB}`;
  return "/account";
}

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);
  /** Stamped with the user it was read for: an admin's landing route depends on
   *  it, and a value fetched for someone else (or for nobody, before sign-in)
   *  must not be mistaken for this user's. */
  const [setupState, setSetupState] = useState<{ userId: number; wizardFinished: boolean } | null>(null);

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

  const needsSetupState = Boolean(me && !me.must_change_password && can(me, "settings.read"));

  useEffect(() => {
    if (!needsSetupState || !me) return;
    const userId = me.id;
    api
      .getSetupState()
      .then((s) => setSetupState({ userId, wizardFinished: s.wizard_finished }))
      // A failed read must not trap an admin on the wizard.
      .catch(() => setSetupState({ userId, wizardFinished: true }));
  }, [me, needsSetupState]);

  // Resolving `/` before the setup state arrives would send a half-configured
  // deployment to the dashboard instead of the wizard, so wait for the read that
  // belongs to this user rather than whatever a previous one left behind.
  if (loading || (needsSetupState && setupState?.userId !== me!.id)) {
    return <div className="flex h-full items-center justify-center text-text-hint">Loading...</div>;
  }

  const home = !me
    ? "/login"
    : me.must_change_password
      ? "/change-password"
      : setupState?.wizardFinished === false && can(me, "settings.write")
        ? "/setup"
        : landingRoute(me);

  // Everything behind the shell shares one gate: signed in, password settled.
  const gate = !me ? (
    <Navigate to="/login" replace />
  ) : me.must_change_password ? (
    <Navigate to="/change-password" replace />
  ) : null;

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
      <Route path="/" element={<Navigate to={home} replace />} />
      <Route
        path="/setup"
        element={
          gate ??
          (!can(me!, "settings.write") ? (
            <Navigate to="/" replace />
          ) : (
            <SetupWizardPage me={me!} onFinish={() => setSetupState({ userId: me!.id, wizardFinished: true })} />
          ))
        }
      />
      <Route element={gate ?? <AppShell me={me!} onLogout={refresh} />}>
        <Route
          path="/dashboard"
          element={can(me!, "dashboard.read") ? <DashboardPage me={me!} /> : <Navigate to="/" replace />}
        />
        {/* Gated here rather than inside each page: reaching a page whose API
            calls all 403 reads as a broken app, and the routing table is the one
            place the whole set is visible. */}
        <Route path="/users" element={can(me!, "users.read") ? <UsersPage me={me!} /> : <Navigate to="/" replace />} />
        <Route path="/roles" element={can(me!, "roles.read") ? <RolesPage me={me!} /> : <Navigate to="/" replace />} />
        <Route path="/workspaces" element={<WorkspacesPage me={me!} />} />
        <Route path="/account" element={<AccountPage me={me!} />} />
        <Route path="/settings" element={<Navigate to={`/settings/${DEFAULT_SETTINGS_TAB}`} replace />} />
        <Route
          path="/settings/:tab"
          element={can(me!, "settings.read") ? <SettingsPage /> : <Navigate to="/" replace />}
        />
      </Route>
      <Route path="/forgot-password" element={<ForgotPasswordPage />} />
      <Route path="/reset-password" element={<ResetPasswordPage />} />
      <Route path="/register" element={<RegisterPage />} />
      <Route path="/verify-email" element={<VerifyEmailPage />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}
