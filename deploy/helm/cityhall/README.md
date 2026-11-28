# CityHall Helm chart

## Requirement: a CNI that enforces NetworkPolicy

With `workspaces.enabled`, the chart ships a `<release>-workspaces` NetworkPolicy
that admits only the CityHall pod to a workspace pod's daemon port 8080. **The
cluster CNI must enforce NetworkPolicy.** If it does not, the API server still
accepts the object and nothing is enforced: every workspace pod can reach every
other workspace's daemon port, protected only by the per-workspace bearer. Full
mode (a shell in the workspace) depends on this isolation. CityHall cannot detect it and the chart has
no value or probe for it.

To check, request port 8080 of one workspace pod from another workspace pod: it
must time out (some CNIs refuse at once instead; either way the connection
must not succeed). An HTTP response, even 401, means the peer is not blocked:
look for another NetworkPolicy that selects the pod, then at the CNI.
See [Workspaces](../../../docs/workspaces.md#network-isolation-between-workspaces).
