# Architecture

## Why a separate `cpu-power-hal` crate

`cpu-power-hal` has no dependency on `tokio`, `kube`, or any async
runtime — it's pure, synchronous, sysfs-in/sysfs-out. This is the "library
that abstracts CPU generation/vendor differences" the project exists to
provide: fast to unit-test (no runtime needed), reusable outside
Kubernetes entirely (see `pstate-cli`), and it keeps "what profile should
this node have" (label/k8s logic, in `power-agent`) cleanly separate from
"how do I make the kernel do that" (mechanism, in `cpu-power-hal`).

## The `PowerBackend` abstraction

```
PowerProfile (enum)          — default/performance/balance_performance/balance_power/power,
                                mapped 1:1 onto the exact kernel EPP sysfs strings

SysfsIo (trait)               — read_to_string / write / read_dir / exists
  RootedSysfs                 — the only impl; rooted at "/" in prod, at a
                                 tempdir in tests, so both exercise real
                                 filesystem semantics

CpuPolicy / discover_policies — enumerates /sys/devices/system/cpu/cpufreq/policy*,
                                 not per-logical-CPU paths (some drivers group
                                 multiple CPUs under one policy)

PowerBackend (trait)           — kind / is_supported / apply / current / set_turbo
  EppBackend                   — Intel intel_pstate (active/HWP) AND AMD amd-pstate-epp,
                                  same struct, same code path (mechanism is
                                  byte-identical at the kernel ABI level);
                                  a Vendor field only matters for set_turbo
  GovernorFallbackBackend      — older/passive intel_pstate, acpi-cpufreq;
                                  collapses the 5 profiles onto whatever
                                  governors scaling_available_governors offers
  UnsupportedBackend           — no cpufreq policies at all (e.g. a QEMU
                                  guest); every operation is a no-op Ok(()),
                                  never an Err

detect_backend()                — picks among the above; never fails
```

`set_turbo(enabled: bool)` is always the positive sense. Each backend
hides its own sign convention: Intel's `intel_pstate/no_turbo` is
inverted (`enabled=true` writes `"0"`), AMD's `cpufreq/boost` is direct
(`enabled=true` writes `"1"`). Callers everywhere else just say
`backend.set_turbo(true)`.

## Why not `kube::runtime::Controller`

Each `power-agent` pod reconciles exactly one object — its own Node —
forever. `Controller` is built for many-object reconciliation with
requeueing/backoff across independent items, which doesn't apply to a
cardinality-1-per-process daemon. A plain `tokio::select!` between the
watch stream and a periodic resync timer (`crates/power-agent/src/reconcile.rs`)
is simpler and equally correct here.

## What was deliberately left out

CRDs (label-driven was the explicit design goal, not a CRD-based operator
like the archived `intel/kubernetes-power-manager`); leader election (no
shared state — each pod owns exactly its own Node); OpenTelemetry tracing
export; a dynamic/pluggable backend registry (exactly 4 backends, all
known at compile time); per-CPU-policy differentiated profiles within one
node; a Helm chart (raw manifests are proportionate for a 7-node cluster).

See `docs/labels.md` for the Node label contract and `docs/verification.md`
for how to validate all of this end-to-end on this specific cluster.
