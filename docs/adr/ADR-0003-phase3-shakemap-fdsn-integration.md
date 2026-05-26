# ADR-0003: Phase 3 — ShakeMap Enrichment and FDSN Federation

**Status**: Accepted  
**Date**: 2026-05-26  
**Deciders**: ChronoCoders

---

## Context

Phase 2 delivers real-time earthquake ingestion from three hard-coded sources (USGS, EMSC, AFAD) and a magnitude-based MMI/felt-radius estimate computed in the analysis service. Two limitations follow:

1. **Coverage**: adding a new seismic network requires a new Rust ingestion adapter — bespoke polling logic, deduplication tuning, and a deployment change per source.
2. **Ground motion accuracy**: the current felt-radius and MMI estimates are derived from magnitude alone (Grünthal 2009 attenuation). They do not account for focal depth variation, local site amplification, or directivity. USGS ShakeMap computes these from station recordings and produces authoritative ground-motion grids.

## Decision

Phase 3 will integrate two data sources:

### 1. USGS ShakeMap Enrichment

- After an event is written to `earthquakes.enriched`, the analysis service will query the USGS ShakeMap API (`https://earthquake.usgs.gov/earthquakes/eventpage/{eventid}/shakemap`) for events M≥3.5.
- Retrieved fields: PGA grid, PGV grid, MMI contour GeoJSON, `mmi_max`, `pga_max`.
- These replace the current `estimated_intensity_mmi` and `estimated_felt_radius_km` fields with authoritative values; the magnitude-only estimates remain as fallback when ShakeMap is unavailable (events < M3.5 or within the ~5 min ShakeMap generation window).
- ShakeMap data is added to a new `shakemap` JSONB column on the `earthquake_events` table and forwarded on `earthquakes.enriched`.

### 2. FDSN Federation Adapter

- A new ingestion mode (`fdsn` source type) implements the [FDSNWS-event 1.1 spec](https://www.fdsn.org/webservices/fdsnws-event-1.1.pdf) as a generic HTTP polling adapter.
- Target networks in Phase 3: **GFZ** (Potsdam), **GEOFON**, **INGV** (Italy), **NIED** (Japan).
- New networks are added via config (`.env` / `config/ingestion/sources.toml`) with fields: `base_url`, `network_code`, `min_magnitude`, `lookback_secs`. No code changes required per network.
- The existing `source_id` deduplication key (`{network}:{event_id}`) prevents duplicate ingestion when the same event is reported by multiple FDSN nodes.

## Consequences

**Positive**
- ShakeMap MMI contours enable real polygon overlays on the map instead of estimated circles — significant UX improvement.
- FDSN adapter reduces per-network engineering cost from ~2 days to a config entry; global coverage becomes achievable.
- No new Kafka topics or schema breaking changes required — ShakeMap data fits in a new nullable JSONB column; FDSN events flow through the existing `earthquakes.raw` → `earthquakes.enriched` pipeline.

**Negative / Trade-offs**
- ShakeMap is only available for USGS-catalogued events; EMSC/AFAD events require a cross-reference lookup by time+location which adds latency and may fail.
- ShakeMap generation takes ~5 minutes after an event; the analysis service must handle the case where ShakeMap is not yet available and retry without blocking the enriched event.
- FDSN polling latency varies by network (NIED: ~1 min; ISC mirror: ~60 days); `lookback_secs` must be tuned per source.

## Alternatives Considered

- **PAGER (fatality/loss estimates)**: deferred — politically sensitive, requires careful UX design, not core to the seismic monitoring mission.
- **SEEDLINK real-time waveform streaming**: deferred to Phase 3 waveform storage work; requires significant DSP infrastructure and storage budget.
- **ISC bulletin**: deferred — 60-day review lag makes it incompatible with real-time display; relevant only for a future historical research mode.
