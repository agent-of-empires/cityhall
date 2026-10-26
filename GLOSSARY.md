# CityHall

CityHall is the multi-user layer over aoe: it gives each user their own aoe workspace and decides what that workspace lets them do.

## Language

**Workspace**:
A user's own aoe instance, run by CityHall on their behalf.
_Avoid_: Container, pod, instance

**Full mode**:
A workspace that serves aoe's complete dashboard, including the terminal, diff panes, worktrees, and yolo. Granted by the user's role.
_Avoid_: Dev tier, unlocked, technical mode

**Composer mode**:
A workspace limited to aoe's composer and structured view, with no terminal or project management. Users whose role lacks Full mode get this.
_Avoid_: PM tier, cityhall mode, locked mode, non-technical mode
