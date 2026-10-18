# cubby

A child-safe, allow-list-only YouTube frontend: a parent control plane plus a child-facing site at cubby.gantt.pub (and, later, an iPad shell).

Design: docs/design/cubby-hld-2026-10-17.md (high-level, approved 2026-10-17).

Developed by Spira (repo label `cubby`, local-only, lands on `local/main`).

## Gate tools

The gate (`deploy/ci.sh`, run in the rust image) provisions its non-Rust tools from the
`gate_tools` list at the top of that script (currently `jq`, `dnsutils` for `dig`). A check
that calls a new tool must add it to that list.
