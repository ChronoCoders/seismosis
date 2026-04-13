# ADR-0002: Frontend Map Library — maplibre-gl over Mapbox GL JS

**Date:** 2026-04-13
**Status:** Accepted
**Deciders:** Phase 2 engineering lead

## Context

The Phase 2 Turkish-language SPA (`frontend/`) requires an interactive map for displaying earthquake epicentres in real time. Two primary candidates were evaluated:

- **Mapbox GL JS** — the original WebGL map renderer; requires an access token for all tile requests.
- **maplibre-gl** — a community fork of Mapbox GL JS v1 (before the license change); fully open-source (BSD-2-Clause); API-compatible with Mapbox GL JS for the subset of features used in this application; no access token required.

The React bindings under evaluation were `react-map-gl`, which supports both backends through a peer dependency.

## Decision

Use `maplibre-gl@4.x` as the map renderer with `react-map-gl@7.1.7` as the React wrapper. Map tiles are sourced from CartoCDN's publicly accessible dark-matter style (`https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json`), which requires no API token.

```json
"react-map-gl": "7.1.7",
"maplibre-gl": "^4.0.0"
```

Import the maplibre variant explicitly:
```tsx
import Map from 'react-map-gl/maplibre';
```

## Rationale

**No access token required.** Mapbox GL JS requires an account-bound access token for all map tile requests. This token must be present in the built JavaScript bundle (it is a `NEXT_PUBLIC_*` variable), is visible to any user who inspects the page, and is subject to Mapbox's per-request billing. maplibre-gl with CartoCDN tiles requires no token and incurs no per-request cost.

**API compatibility.** For the features used in this application — GeoJSON sources, circle layers, layer expressions, popups, NavigationControl, ScaleControl — the maplibre-gl API is identical to Mapbox GL JS. No application-level code differs between the two backends.

**License.** maplibre-gl is BSD-2-Clause. Mapbox GL JS v2+ is proprietary (Mapbox ToS). Using a proprietary dependency for an open-source platform component is undesirable.

**GeoJSON Source + Layer over individual Markers.** Earthquake events are rendered as a GeoJSON FeatureCollection with two `circle` layers (a live-ring layer filtered on `is_live=true`, and a base circle layer). This is significantly more performant than mounting individual React `<Marker>` components for each event: GeoJSON sources update as a single WebGL draw call regardless of event count, while marker-per-event creates one DOM element per event. With up to 100 events in the dashboard state, the DOM overhead of individual markers is not catastrophic, but the GeoJSON approach scales to thousands of events without degradation.

## Consequences

- No Mapbox API key is required at build time or runtime.
- CartoCDN's tile service is a third-party dependency with no SLA commitment to Seismosis. If CartoCDN's dark-matter endpoint is unavailable, the map renders without tiles (events still plotted correctly on a blank background). For Phase 3 production, self-hosting or a contracted tile provider should be considered.
- `maplibre-gl` accesses `window` at module load time and cannot render server-side. All components that import it must use `dynamic(() => import(...), { ssr: false })`. This is applied in `Dashboard.tsx` and `/deprem/[id]/page.tsx`.
- `NEXT_PUBLIC_MAP_STYLE` is accepted as a Docker build ARG, defaulting to the CartoCDN URL. It can be overridden at build time to point to a self-hosted or contracted tile service without code changes.

## Alternatives Considered

**Mapbox GL JS v2+:** Requires access token, proprietary license, per-request billing. Rejected on all three grounds.

**Leaflet.js:** Open-source, lower WebGL requirements. The plugin ecosystem for large GeoJSON datasets and real-time updates is weaker than maplibre-gl. Rejected in favour of maplibre-gl's native GeoJSON layer support.

**deck.gl:** Excellent for large-scale geospatial rendering. Significantly more complex API for the simple circle-layer use case here. Rejected as overengineered for Phase 2.
