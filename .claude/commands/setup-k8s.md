Deploy radar-ng to Kubernetes and verify it. Target or extra context: $ARGUMENTS

Source of truth: `deploy/kubernetes/README.md` (read it first). Production background:
`docs/self-host/kubernetes.md`.

1. **Which cluster?** Run `kubectl config current-context` and **confirm with the user** before
   applying anything. Never apply to a cluster managed by GitOps (Argo CD / Flux): there, the change
   goes in a PR to that GitOps repo instead.
2. **Prerequisites:** `kubectl version` (1.27+), `kubectl get storageclass` shows one `(default)`
   class, and the nodes have outbound internet (NOAA, Protomaps, GitHub).
3. **Settings:** set `NWS_USER_AGENT` in `deploy/kubernetes/base/settings.env` (or in the user's own
   overlay, preferred) to `(radar-ng, <their email>)`; **ask for the email**.
4. **Render first:** `deploy/kubernetes/validate.sh` (or `kubectl kustomize deploy/kubernetes/examples/minimal`).
   Fix render errors before applying.
5. **Apply:** `kubectl apply -k deploy/kubernetes/examples/minimal` for a test cluster. For anything
   longer-lived, create an overlay as in the README's "Make it your own" section.
6. **Verify** (poll; give up after ~10 minutes):
   - `kubectl -n radar-ng get pods`: everything `Running`/`Completed`.
   - `kubectl -n radar-ng port-forward svc/tile-server 8080:8080`, then `curl -s localhost:8080/api/livez`
     and `curl -s localhost:8080/api/health` (`degraded` → `ok` once the first frame lands).
7. **If not:** use the README's "If it didn't work" table, then `kubectl -n radar-ng logs` on the failing
   pod. The `doctor` skill gives a full health sweep.
8. **Report** pod status, the health output, and anything you skipped.

Things that must stay true are listed in the README's "Things that must stay true" section; don't
change probes, uids or mount paths to make something start.
