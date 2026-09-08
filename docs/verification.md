# Verification plan

Written against this cluster's actual topology (`kubectl get nodes -o
wide`, Talos v1.13.9, Kubernetes v1.35.8):

| Node                       | Role           | Hardware                          |
|----------------------------|----------------|--------------------------------------|
| `<node-storage>`           | storage        | Intel                               |
| `<node-worker-1>`          | worker         | Intel                               |
| `<node-worker-2>`          | worker         | Intel                               |
| `<node-control-plane-1>`   | control-plane  | Intel                              |
| `<node-control-plane-2>`   | control-plane  | Intel                              |
| `<amd-framework-desktop>`  | (none)         | AMD Framework Desktop (Strix Halo) |
| `<qemu-vm>`                | control-plane  | QEMU VM (cordoned/`SchedulingDisabled`) |

Two notes this changes versus a generic plan:

- **All 5 Intel nodes are the same generation** (Alder Lake), not a mix as
  originally assumed — vendor extension labels confirm this. They should
  all detect as the `intel_epp` backend; the `governor_fallback` path is
  defensive/future-proofing for this fleet rather than something you'll
  actually see today. `pstate-cli probe` already confirmed this detection
  path works correctly against a real machine with this same
  `intel_pstate`+EPP shape.
- **Talos has no SSH and no shell on the host.** Use `kubectl debug
  node/<name>` (schedules a normal debug pod with `/host` mounted — works
  identically on Talos to any other distro, since it's a Kubernetes-native
  mechanism, not a host feature) or `talosctl read
  /sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference
  --nodes <IP>` if you have `talosctl` configured, instead of `ssh`+`cat`.
- **The QEMU VM is already cordoned** (`node.kubernetes.io/unschedulable:NoSchedule`).
  DaemonSet pods tolerate this taint automatically by default (and this
  project's blanket `tolerations: [{operator: Exists}]` covers it too),
  so the agent pod will still land there — that's expected, not a bug.

## Steps

1. Build and push the image to a registry your cluster can pull from,
   update `image:` in `deploy/manifests/04-daemonset.yaml`, then:
   ```bash
   kubectl apply -f deploy/manifests/
   kubectl get pods -n cpu-power-system -o wide
   ```
   Confirm 7/7 pods `Running` — this alone validates the QEMU node's
   no-op path before any label is even applied.

2. **Primary target end-to-end**:
   ```bash
   kubectl label node <node-storage> <node-worker-1> <node-worker-2> <node-control-plane-1> <node-control-plane-2> \
     cpu-power.io/profile=power cpu-power.io/turbo=disabled

   kubectl label node <amd-framework-desktop> \
     cpu-power.io/profile=performance cpu-power.io/turbo=enabled
   ```
   Tail each pod's logs (`kubectl logs -n cpu-power-system <pod> -f`) and
   confirm a successful reconcile on all 6 labeled nodes.

3. On an Intel node, confirm via a debug pod:
   ```bash
   kubectl debug node/<node-control-plane-1> -it --image=busybox -- \
     sh -c 'cat /host/sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference; \
            cat /host/sys/devices/system/cpu/intel_pstate/no_turbo'
   ```
   Expect `power` and `1` (turbo disabled — remember `no_turbo=1` means
   *disabled*). Cross-check against
   `kubectl get node <node-control-plane-1> -o jsonpath='{.metadata.annotations}'`.

4. On the Framework Desktop:
   ```bash
   kubectl debug node/<amd-framework-desktop> -it --image=busybox -- \
     sh -c 'cat /host/sys/devices/system/cpu/cpufreq/policy0/energy_performance_preference; \
            cat /host/sys/devices/system/cpu/cpufreq/boost 2>/dev/null || \
            cat /host/sys/devices/system/cpu/amd_pstate/boost'
   ```
   Expect `performance` and `1` (turbo enabled, direct mapping this time —
   confirms the Intel/AMD inversion is handled correctly on real hardware,
   not just in fixtures). Confirm the annotation shows
   `cpu-power.io/backend: amd_epp`, not `governor_fallback` — this is the
   one thing that genuinely needs validating on real silicon rather than
   fixtures, since `amd-pstate-epp`'s exact sysfs surface has moved around
   across kernel versions.

5. Drift correction: from a debug pod, overwrite a value directly
   (`echo balance_power > /host/sys/.../energy_performance_preference`),
   wait past `RESYNC_INTERVAL_SECONDS` (300s default), confirm the agent
   puts it back on its own.

6. `<qemu-vm>`: confirm its pod stays `Running` and its annotation shows
   `cpu-power.io/status: unsupported` regardless of labeling.

7. Remove a profile label → confirm fallback to `default`. Apply
   `cpu-power.io/profile=ultra` to a test node → confirm a `Warning` Event
   (`kubectl get events -n cpu-power-system --field-selector reason=InvalidProfileLabel`)
   and safe fallback, no crash.

8. `kubectl port-forward -n cpu-power-system <pod> 9090:9090`, then `curl
   localhost:9090/metrics` and `/healthz`, confirm expected series and a
   200.
