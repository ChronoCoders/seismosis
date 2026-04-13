# Session: 2026-04-13 — Phase 2 Frontend + WebSocket Fixes

## Goal

Apply all rust-reviewer findings to the WebSocket service (`services/websocket/`) and deliver the final Phase 2 deliverable: the Turkish-language SPA (`frontend/`). After this session, all three Phase 2 application components (analysis, WebSocket, frontend) are code-complete.

This document is written for engineers who were not present. It assumes familiarity with the Phase 2 analysis service session at `docs/sessions/2026-04-12-phase2-analysis-service.md` but no other shared context.

---

## What Was Done

### Commit `bb614dc` — rust-reviewer fixes to `services/websocket/`

The rust-reviewer agent identified 3 blockers and 8 warnings in the previously-committed WebSocket service. All 11 findings were resolved in this commit. See the Decisions Made section for the precise fixes.

### Commit `9f83eb6` — Phase 2 Next.js Turkish frontend (`frontend/`)

24 new files implementing the complete Turkish-language SPA. See Files Changed for the full list.

---

## Decisions Made

### WebSocket service — Blocker fixes

**BLOCKER-1: Kafka offset management**

The original code set `enable.auto.commit=true` without `enable.auto.offset.store=false`. This causes librdkafka to commit offsets for messages the application has not yet processed, because the auto-store fires on message delivery from the broker, not on application acknowledgement.

Fixed by setting:
```
enable.auto.commit=true
enable.auto.offset.store=false
auto.commit.interval.ms=5000
```
After each message is fully processed (or skipped on error), the service calls `consumer.store_offset_from_message(&msg)`. The auto-commit timer at 5 s flushes stored offsets to Kafka in the background, keeping per-message overhead off the hot path. This is the correct rdkafka pattern for at-least-once delivery.

See ADR-0001 for the full decision record.

**BLOCKER-2: HTTP client reuse in `pre_warm_cache`**

`pre_warm_cache` was constructing a bare `reqwest::Client` internally. This bypassed the shared client built in `main`, which has a 10 s timeout and a `User-Agent` header configured. It also prevented connection pool reuse between the warm-up requests and the Schema Registry cache fetches that follow.

Fixed by changing the signature to accept `client: &reqwest::Client`. The shared client from `main` is passed in, and all Schema Registry HTTP calls in the service now use the same pooled client.

**BLOCKER-3: Shutdown race between `hub.close_all()` and `accept_handle.await`**

The original shutdown sequence called `hub.close_all()` before awaiting the accept loop. New clients accepted in the window between the shutdown signal and the accept loop stopping would not receive a `Close` frame.

Fixed shutdown sequence in `main`:
1. `shutdown_tx.send(true)` — broadcast shutdown to all tasks.
2. `accept_handle.await` — the accept loop internally drives a `JoinSet` of per-client tasks; awaiting it blocks until all in-flight clients have exited cleanly.
3. `hub.close_all()` — now acts as a safety net for the narrow connection window; hub is empty in normal operation by the time this runs.
4. `consume_handle.await` / `metrics_handle.await` — remaining tasks.

### WebSocket service — Warning fixes

**WARN-1: Mutex poison handling in `SchemaCache`**

Both the fast-path lock and the insert-path lock in `avro.rs` replaced `.lock().unwrap()` with `.lock().map_err(|_| WsError::SchemaRegistry { ... })?`. Mutex poisoning is now surfaced as a typed error rather than a panic.

**WARN-2: Arc removed from query capture in `handler.rs`**

The `accept_hdr_async` callback fires synchronously during the WebSocket handshake (before the future resolves). The captured query string was wrapped in `Arc<Mutex<String>>`, which is unnecessary because there is no concurrent access — only the callback writes and only the post-await code reads. Replaced with a stack-allocated `std::sync::Mutex<String>`.

**WARN-3: Hot-path clone in hub broadcast loops**

In `hub.rs`, `broadcast_enriched` and `broadcast_alert` previously called `Arc::clone(&msg)` before evaluating the per-client filter. For messages filtered by most clients (e.g. high-magnitude-only subscribers), this cloned the Arc unnecessarily for every client.

Fixed by extracting the inner event reference once before the loop using an irrefutable let-else pattern, then passing the extracted reference to the filter predicate. `Arc::clone` is called only for clients that pass the filter.

**WARN-4: Shutdown watch initial state**

`tokio::sync::watch::Receiver::changed()` only resolves on value *transitions*. If a per-client handler task starts after the shutdown signal is broadcast, the initial state is already `true` but `changed()` will never fire. Added an explicit `if *shutdown.borrow() { return; }` guard at loop entry in both the accept loop and the per-client handler.

**WARN-5: `JoinSet` for client tasks**

The accept loop replaced untracked `tokio::spawn` calls with `JoinSet<()>`. At shutdown, the accept loop drives `join_next()` to drain all in-flight client tasks and log any per-task panics before returning. This ensures `hub.close_all()` in step 3 above runs only after all clients have cleanly exited.

**WARN-6: `#[tracing::instrument]` on key async functions**

Added to `pre_warm_cache`, `run_consume_loop`, `run_accept_loop`, and `handle_connection` to propagate span context through structured logs.

**WARN-7: Percent-decoding for query parameter values**

Filter query parameters are now run through `percent_encoding::percent_decode_str(...).decode_utf8_lossy()` before parsing. This handles cases like `source_networks=US%2CJP` (which should decode to `US,JP`). Added `percent-encoding = "2"` to `services/websocket/Cargo.toml`.

**WARN-8: `None` from `to_json()` logged**

`ServerMessage::to_json()` returns `None` for the `Close` variant (close frames are handled separately). The outbound arm of the client select loop previously skipped `None` silently. Added a `warn!` log when `to_json()` returns `None` for a non-`Close` message, surfacing any future `ServerMessage` variants that forget to implement `to_json()`.

### Frontend — Architecture decisions

See ADR-0002 for maplibre-gl over Mapbox GL JS.

**Next.js rewrites as CORS proxy**

`frontend/next.config.ts` configures `rewrites()` to proxy `/api/v1/*` to the Rust API service. Client-side `fetchEvents` / `fetchEvent` / `fetchStats` call `/api/v1/...` (same-origin). Server-side (SSR for the detail page) calls `http://seismosis-api:8000/v1/...` directly via the internal Docker network. This eliminates any CORS configuration requirement on the Rust API, accepting the trade-off that the Next.js server adds one hop for REST calls on the client path.

**`output: 'standalone'` for Docker builds**

`next.config.ts` sets `output: 'standalone'`. This produces a self-contained `server.js` + minimal `node_modules` in `.next/standalone`, which the Dockerfile copies directly into the runtime image. The result is a significantly smaller Docker layer than copying all `node_modules` into the final stage.

**Custom SVG bar chart over recharts**

The magnitude distribution chart in `RegionalStats.tsx` is a custom SVG implementation rather than using recharts or another charting library. The chart has only one dimension (count per magnitude band) and no interactivity requirements beyond a hover label. Adding recharts (~350 KB compressed) for a simple bar chart was rejected on bundle size grounds.

**`dynamic()` with `ssr: false` for map components**

`MapInner.tsx` imports `maplibre-gl`, which accesses `window` and the DOM at module load time. It is imported via `dynamic(() => import('@/components/MapInner'), { ssr: false })` in both `Dashboard.tsx` and the detail page (`/deprem/[id]/page.tsx`) to prevent SSR from attempting to render the map on the server.

**WebSocket auto-reconnect at 5 s**

`useWebSocket.ts` schedules a reconnect in the `onclose` handler with a fixed 5 s delay. There is no exponential backoff in this implementation. This is acceptable for Phase 2 where the WebSocket service is single-instance and restarts are expected to be brief. **TECH DEBT:** If the WebSocket service has planned maintenance windows or rolling restarts under Phase 3 Kubernetes, the reconnect strategy should use exponential backoff with jitter to avoid reconnect storms.

**Alert auto-dismiss at 60 s**

Alert banners displayed in `Dashboard.tsx` are automatically dismissed after 60 seconds via a `setTimeout`. The timer is cleared and restarted if a new alert arrives while an existing banner is showing, so the banner always shows for a full 60 s from the most recent alert.

**Magnitude colour coding**

Defined in `frontend/src/lib/magnitude.ts`:

| Range | Colour | Turkish label |
|-------|--------|---------------|
| M < 3 | `#22c55e` (green) | Hafif |
| 3 ≤ M < 5 | `#eab308` (yellow) | Orta |
| 5 ≤ M < 7 | `#f97316` (orange) | Kuvvetli |
| M ≥ 7 | `#ef4444` (red) | Siddetli |

These thresholds match the alert levels produced by `services/analysis/src/analysis/risk.py`.

**Map default view**

`MapInner.tsx` initialises with `longitude: 35.0, latitude: 39.0, zoom: 4.5` — centred on Turkey. The map style is `https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json` (CartoCDN hosted, publicly accessible, no API token required).

**`NEXT_PUBLIC_WS_URL` embedded at build time**

`NEXT_PUBLIC_*` variables in Next.js are inlined into the JavaScript bundle at build time. `NEXT_PUBLIC_WS_URL` is accepted as a Docker build ARG defaulting to `ws://localhost:9093`. In `docker-compose.yml` the default is used, which means the browser connects directly to the host-exposed port 9093. For production deployment on Vultr, this ARG must be overridden to the public hostname/port before building the image.

**`docker-compose.yml` frontend service**

Added `seismosis-frontend` on host port 3001 (container port 3000). Port 3001 avoids conflict with Grafana on 3000. `depends_on` requires `seismosis-api` and `seismosis-websocket` to be healthy before the frontend starts.

---

## Open Items

### 1. Memory limits required before Vultr deployment (TD-001) — carried forward

No service in `docker-compose.yml` has `deploy.resources.limits.memory`. This is a hard blocker for production deployment. All services (including the new WebSocket and frontend services added this session) must be profiled under representative load before limits can be set.

**Owner:** Infrastructure / deployment lead
**Target:** Before Vultr deployment
**Blocking:** Yes

### 2. Frontend not tested against live backend

The complete Phase 2 stack (analysis + WebSocket + frontend + all infrastructure) has not been started together during development. The frontend was built and is logically correct against the documented API and WebSocket contracts, but end-to-end integration testing has not been performed.

**Owner:** Phase 2 lead
**Target:** First available full-stack run
**Blocking:** Yes, before Phase 2 is declared complete

### 3. `NEXT_PUBLIC_WS_URL` must be overridden for Vultr deployment

The value is embedded in the bundle at build time. The `docker-compose.yml` default (`ws://localhost:9093`) is correct for local development but incorrect for production where the WebSocket service will be behind a public hostname. The deployment process must set this build ARG before building the frontend image.

**Owner:** Infrastructure / deployment lead
**Target:** Before Vultr deployment

### 4. WebSocket auto-reconnect uses fixed 5 s delay — no backoff

See the decision note above. This is acceptable for Phase 2 single-instance deployment.

**TECH DEBT:** Add exponential backoff with jitter to `useWebSocket.ts` before any Phase 3 topology that involves planned WebSocket service restarts.

**Owner:** Frontend lead
**Target:** Phase 3 planning

### 5. Grafana dashboard not updated for Phase 2 metrics — carried forward

`config/grafana/dashboards/earthquake_overview.json` has no panels for `seismosis_analysis_*` metrics (9 metrics) or `seismosis_websocket_*` metrics (8 metrics). Both sets are being scraped by Prometheus but are invisible in Grafana.

**Owner:** Phase 2 lead
**Target:** Before Phase 2 production launch

### 6. `docs/sessions/2026-04-12-phase2-analysis-service.md` is untracked

This file was written during the previous session but was not committed. It should be committed to `main` in a standalone commit before the next session begins.

**Owner:** Phase 2 lead
**Target:** Immediately

### 7. `seismology.event_station_associations` table ownership unresolved — carried forward

See `docs/sessions/2026-04-12-phase2-analysis-service.md` — Open Item 2 for full context.

### 8. `SignificantEarthquakeDetected` alert annotation still incorrect — carried forward

See `docs/sessions/2026-04-12-phase2-analysis-service.md` — Open Item 3. Expression is correct; annotation summary text reads "M≥5.0" but should read "M≥7.0".

---

## Files Changed

### Modified — `services/websocket/`

| File | Change |
|------|--------|
| `services/websocket/src/main.rs` | Kafka config: added `enable.auto.offset.store=false`; `store_offset_from_message` after each message; fixed shutdown order (accept → hub.close_all → consume → metrics); `pre_warm_cache` now accepts `&reqwest::Client`; `#[tracing::instrument]` on key functions |
| `services/websocket/src/avro.rs` | `SchemaCache::get()` fast-path and insert-path: `.lock().unwrap()` → `.lock().map_err(...)? ` |
| `services/websocket/src/handler.rs` | Query capture: `Arc<Mutex<String>>` → stack `std::sync::Mutex<String>`; added shutdown-on-entry guard; added `warn!` for `to_json()` returning `None` on non-Close message |
| `services/websocket/src/hub.rs` | `broadcast_enriched` / `broadcast_alert`: extract inner ref before loop with irrefutable let-else; `Arc::clone` only for filter-passing clients |
| `services/websocket/src/filter.rs` | `from_query`: query values now percent-decoded via `percent_encoding::percent_decode_str` |
| `services/websocket/Cargo.toml` | Added `percent-encoding = "2"` |

### New — `frontend/`

| File | Description |
|------|-------------|
| `frontend/package.json` | Package metadata; dependencies: Next.js 14, TypeScript, Tailwind CSS, react-map-gl@7.1.7, maplibre-gl@4.x |
| `frontend/next.config.ts` | `output: 'standalone'`; `rewrites()` proxying `/api/v1/*` to Rust API |
| `frontend/tailwind.config.ts` | Custom colour tokens: `mag-red`, `mag-orange`, `mag-yellow`, `mag-green`, `surface`, `border`, `text-primary`, `text-secondary`, `text-muted` |
| `frontend/tsconfig.json` | TypeScript config; path alias `@/` → `src/` |
| `frontend/Dockerfile` | Multi-stage: `node:20-alpine` deps → builder → runner; non-root `nextjs` user (uid 1001); `NEXT_PUBLIC_*` as build ARGs; `output: standalone` |
| `frontend/src/types/index.ts` | Shared TypeScript types: `EarthquakeEvent`, `EnrichedEvent`, `AlertEvent`, `WsMessage`, `DisplayEvent`, `BandStats`, `EventListResponse`, `StatsResponse`; adapter functions `apiEventToDisplay`, `enrichedToDisplay` |
| `frontend/src/lib/magnitude.ts` | `getMagnitudeInfo()`; colour + label lookup by magnitude; `formatMagnitude`, `formatDepth`, `formatRelativeTime`, `formatDateTime`, `formatDateTimeMs` |
| `frontend/src/lib/api.ts` | `fetchEvents`, `fetchEvent`, `fetchStats`; dual-mode base URL (client: `/api/v1`, server: `API_URL` env var) |
| `frontend/src/hooks/useWebSocket.ts` | `useWebSocket(url)` hook; auto-reconnect at 5 s fixed delay; exposes `status` and `lastMessage` |
| `frontend/src/app/layout.tsx` | Root layout; dark background; Turkish `<html lang="tr">`; Inter font |
| `frontend/src/app/page.tsx` | Root route; renders `<Dashboard />` |
| `frontend/src/app/deprem/[id]/page.tsx` | SSR event detail page; `generateMetadata`; detail table + mini map; `notFound()` on 404 |
| `frontend/src/components/Dashboard.tsx` | Main client component; WebSocket + REST merge; alert banner with 60 s auto-dismiss; stats polling every 60 s; max 100 live events in state |
| `frontend/src/components/MapInner.tsx` | MapLibre map; GeoJSON Source + two layers (live-ring + circle); hover popup; click navigates to detail page; centred on Turkey |
| `frontend/src/components/Header.tsx` | Service header; WebSocket status indicator; last-updated timestamp |
| `frontend/src/components/EarthquakeList.tsx` | Scrollable event list; magnitude badge + region + relative time per row |
| `frontend/src/components/MagnitudeBadge.tsx` | Reusable magnitude badge; `size` prop (`sm`/`md`/`xl`); optional label |
| `frontend/src/components/RegionalStats.tsx` | `RegionalStats` (stat row per magnitude band) + `MagnitudeChart` (custom SVG bar chart, no recharts dependency) |

### Modified — infrastructure

| File | Change |
|------|--------|
| `docker-compose.yml` | Added `seismosis-frontend` service on port 3001; `NEXT_PUBLIC_WS_URL=ws://localhost:9093`; `depends_on` api (healthy) + websocket (healthy) |
