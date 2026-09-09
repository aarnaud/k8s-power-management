# Node label contract

`power-agent` is driven entirely by two Node labels — no CRDs. Each
DaemonSet pod watches only its own Node (via the `NODE_NAME` Downward API
field) and reconciles whenever these labels change, plus on a periodic
resync (`RESYNC_INTERVAL_SECONDS`, default 300s) that self-heals drift.

## `cpu-power.io/profile`

One of the five values the Linux kernel's `energy_performance_preference`
sysfs attribute accepts:

| Value                 | Meaning                                   |
|------------------------|--------------------------------------------|
| `default`              | Kernel/driver default EPP                  |
| `performance`          | Maximum performance                        |
| `balance_performance`  | Favor performance                          |
| `balance_power`        | Favor power savings                        |
| `power`                | Maximum power savings                      |

- **Absent**: treated as `default`. Normal state for an unmanaged node, not
  a warning.
- **Present but not one of the five values above**: falls back to
  `default`, logged at `warn`, and a `Warning` Event
  (`InvalidProfileLabel`) is recorded on the Node.
- On hardware without EPP support (older/passive-mode `intel_pstate`, or
  generic `acpi-cpufreq`), the agent falls back to `scaling_governor`
  control, which only distinguishes `performance` and `power`; `default`,
  `balance_performance`, and `balance_power` collapse onto the same shared
  governor on that hardware. Check the `cpu-power.io/backend` annotation
  (see below) to see whether a given node is on the full EPP path or the
  coarser governor-fallback path.
- On hosts with no cpufreq exposed at all (e.g. a QEMU guest without host
  CPU passthrough), this label is accepted but has no effect — the agent
  reports `cpu-power.io/status: unsupported` rather than erroring.

## `cpu-power.io/turbo`

One of `enabled` / `disabled`. Independent of the profile label — it
controls the separate turbo/boost-clock ceiling, not the EPP scheduling
hint. On Intel this maps (inverted) to `intel_pstate/no_turbo`; on AMD it
maps directly to `cpufreq/boost` (or `amd_pstate/boost` if present). You
never need to think about that inversion — the label semantics are always
the positive sense ("turbo is allowed to engage").

- **Absent or invalid**: turbo is left untouched (unmanaged) — there's no
  natural "default" turbo state to force the way `default` is a natural
  fallback profile. An invalid value also records a `Warning` Event
  (`InvalidTurboLabel`).
- **Not supported by the detected backend** (e.g. the governor-fallback
  path, which has no vendor-specific boost knob to drive): reported via
  `cpu-power.io/applied-turbo: unsupported`, logged, and does **not**
  count as a reconcile error.

## Example: this cluster's primary target

```bash
# All Intel nodes into power-save mode, turbo off
kubectl label node <node-1> <node-2> <node-3> <node-4> <node-5> \
  cpu-power.io/profile=power cpu-power.io/turbo=disabled

# AMD Framework Desktop into performance mode, turbo on
kubectl label node <amd-framework-desktop> \
  cpu-power.io/profile=performance cpu-power.io/turbo=enabled

# The QEMU control-plane VM is left unlabeled; its agent pod reports
# "unsupported" and does nothing, safely.
```

## Status surfaced back on the Node

When they change, the agent patches these annotations (JSON merge patch
on `.metadata.annotations` only — no `nodes/status` RBAC needed):

- `cpu-power.io/applied-profile`
- `cpu-power.io/applied-turbo` — `enabled` / `disabled` / `unmanaged` / `unsupported`
- `cpu-power.io/backend` — `intel_epp` / `amd_epp` / `unknown_epp` / `governor_fallback` / `unsupported`
- `cpu-power.io/status` — `ok` / `invalid_label` / `unsupported` / `error`

The patch is skipped entirely when these values already match what's on
the Node — deliberately, since this agent watches its own Node, and a
patch that always changed something (e.g. a timestamp) would re-trigger
its own watch stream forever. "When did this last run" is answered by the
`power_agent_last_reconcile_timestamp_seconds` Prometheus metric instead,
not a Node annotation.

Both label keys and the resync interval are configurable via the
`PROFILE_LABEL_KEY`, `TURBO_LABEL_KEY`, and `RESYNC_INTERVAL_SECONDS`
environment variables on the DaemonSet container, if you'd rather use a
different label domain.
