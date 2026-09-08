# k8s-power-management

A DaemonSet that watches Kubernetes Node labels and applies CPU power
management (Intel and AMD, EPP-aware with a governor-based fallback for
older hardware) — no CRDs, no vendor lock-in.

Built because the one project that did this
([`intel/kubernetes-power-manager`](https://github.com/intel/kubernetes-power-manager))
was archived by Intel in September 2024, and its actively-maintained fork
([`AMDEPYC/kubernetes-power-manager`](https://github.com/AMDEPYC/kubernetes-power-manager))
was re-scoped to AMD EPYC only. See `docs/architecture.md` for the full
rationale and design.

## Layout

```
crates/
  cpu-power-hal/   library: vendor/generation-agnostic sysfs power management
  power-agent/     binary: the DaemonSet — watches its own Node, applies profiles
  pstate-cli/      binary: run directly on a node to probe/apply/read without k8s
deploy/manifests/  namespace, RBAC, DaemonSet
docs/              architecture.md, labels.md, verification.md
```

## Quickstart

```bash
cargo test --workspace          # 38 tests, no real hardware required
cargo build --release -p pstate-cli
./target/release/pstate-cli probe    # inspect this machine's own CPU power backend

kubectl apply -f deploy/manifests/
kubectl label node <a-node> cpu-power.io/profile=power cpu-power.io/turbo=disabled
```

The DaemonSet image (`ghcr.io/aarnaud/k8s-power-management`) is built and
pushed automatically by `.github/workflows/docker-publish.yml` on every
push to `main`. GHCR packages default to private even on a public repo —
either flip the package to public under GitHub package settings, or add
an `imagePullSecret` to the `cpu-power-system` namespace, before
`deploy/manifests/04-daemonset.yaml` will actually be able to pull it.

To build the image locally instead:

```bash
docker build -t ghcr.io/aarnaud/k8s-power-management:latest .
docker push ghcr.io/aarnaud/k8s-power-management:latest
```

See `docs/labels.md` for the full label contract and `docs/verification.md`
for an end-to-end validation walkthrough.
