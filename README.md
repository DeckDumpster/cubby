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

## Deploy: creating the first family

Cubby has no self-service signup: a parent can use the parent UI only once a family exists for
their Pocket ID email (otherwise every parent route answers 403 "not a parent of any family").
Provision one from inside the running container (the registry and family file are created
together; the email is matched case-insensitively):

    podman exec <container> cubby-server family create parent@example.com
    podman exec <container> cubby-server family list

Alternatively set `CUBBY_BOOTSTRAP_PARENT=parent@example.com` in `server.env`: at startup, if the
registry holds no families yet, one is created for that email (later restarts are no-ops).
Existing families, including ones inserted by hand, are untouched.

## Icons

The rendered icons in `web/static/assets/icons/` are committed, and nothing in the build produces
them. Their source geometry is `web/icons-src/mark.svg` (rounded: favicon, PWA icons) and
`mark-bleed.svg` (full-bleed: apple-touch, maskable). To change them, re-export the PNG/ICO sizes
from those SVGs with any rasteriser (e.g. `rsvg-convert -w 512 mark.svg`), keep the `-v1` filename
suffix bumped for cache-busting, and commit the results. There is no Python in this repo.
