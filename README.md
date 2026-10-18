# cubby

A child-safe, allow-list-only YouTube frontend: a parent control plane plus a child-facing site at cubby.gantt.pub (and, later, an iPad shell).

Design: docs/design/cubby-hld-2026-10-17.md (high-level, approved 2026-10-17).

Developed by Spira (repo label `cubby`, local-only, lands on `local/main`).

## Production

Cubby runs on **ubuntu-mini-0** (user `ryangantt`), never on the Spira box (agent-swarm-test).
Production is `~/cubby-prod` there; `deploy/deploy.sh` runs from it and refuses on any host other
than the one named in `deploy/prod-host`. Cubby has no git remote, so `deploy/push-prod.sh` ships
`local/main` as a git bundle over ssh into `~/cubby-prod` and runs the deploy.

## Gate tools

The gate (`deploy/ci.sh`, run in the rust image) provisions its non-Rust tools from the
`gate_tools` list at the top of that script (currently `jq`, `dnsutils` for `dig`). A check
that calls a new tool must add it to that list.
