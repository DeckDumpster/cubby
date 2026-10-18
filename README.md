# cubby

A child-safe, allow-list-only YouTube frontend: a parent control plane plus a child-facing site at cubby.gantt.pub (and, later, an iPad shell).

Design: docs/design/cubby-hld-2026-10-17.md (high-level, approved 2026-10-17).

Developed by Spira (repo label `cubby`, local-only, lands on `local/main`).

## Gate tools

The gate (`deploy/ci.sh`, run in the rust image) provisions its non-Rust tools from the
`gate_tools` list at the top of that script (currently `jq`, `dnsutils` for `dig`). A check
that calls a new tool must add it to that list.

## Deploy: creating the first family

Cubby has no self-service signup: a parent can use the parent UI only once a family exists for
their Access email (otherwise every parent route answers 403 "not a parent of any family").
Provision one from inside the running container (the registry and family file are created
together; the email is matched case-insensitively):

    podman exec <container> cubby-server family create parent@example.com
    podman exec <container> cubby-server family list

Alternatively set `CUBBY_BOOTSTRAP_PARENT=parent@example.com` in `server.env`: at startup, if the
registry holds no families yet, one is created for that email (later restarts are no-ops).
Existing families, including ones inserted by hand, are untouched.
