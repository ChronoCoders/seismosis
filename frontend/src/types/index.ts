// ── REST API types ────────────────────────────────────────────────────────────

export interface EarthquakeEvent {
  id: string;
  source_id: string;
  source_network: string;
  event_time: string;       // ISO 8601
  latitude: number;
  longitude: number;
  depth_km?: number;
  magnitude: number;
  magnitude_type: string;
  region_name?: string;
  quality_indicator: string;
  processed_at: string;     // ISO 8601
  pipeline_version: string;
}

export interface EventListResponse {
  events: EarthquakeEvent[];
  page: number;
  page_size: number;
  total: number;
}

export interface BandStats {
  band: string;
  min_magnitude: number;
  max_magnitude?: number;
  count_1h: number;
  count_24h: number;
  count_7d: number;
  count_30d: number;
  max_mag_1h?: number;
  max_mag_24h?: number;
  max_mag_7d?: number;
  max_mag_30d?: number;
}

export interface StatsResponse {
  bands: BandStats[];
  computed_at: string;
}

// ── WebSocket message types ───────────────────────────────────────────────────

export interface ConnectedMessage {
  type: 'connected';
  client_id: string;
}

export interface EnrichedEvent {
  type: 'earthquake';
  source_id: string;
  source_network: string;
  event_time_ms: number;
  latitude: number;
  longitude: number;
  depth_km: number | null;
  magnitude: number;
  magnitude_type: string;
  region_name: string | null;
  quality_indicator: string;
  ingested_at_ms: number;
  pipeline_version: string;
  ml_magnitude: number;
  ml_magnitude_source: string;
  is_aftershock: boolean;
  mainshock_source_id: string | null;
  mainshock_magnitude: number | null;
  mainshock_distance_km: number | null;
  mainshock_time_delta_hours: number | null;
  estimated_felt_radius_km: number;
  estimated_intensity_mmi: number;
  enriched_at_ms: number;
  analysis_version: string;
}

export interface AlertEvent {
  type: 'alert';
  source_id: string;
  event_time_ms: number;
  latitude: number;
  longitude: number;
  depth_km: number | null;
  magnitude: number;
  ml_magnitude: number;
  region_name: string | null;
  estimated_intensity_mmi: number;
  estimated_felt_radius_km: number;
  is_aftershock: boolean;
  alert_level: string;  // "YELLOW" | "ORANGE" | "RED"
  triggered_at_ms: number;
}

export type WsMessage = ConnectedMessage | EnrichedEvent | AlertEvent;

// ── Normalised display type (API event or WS enriched event) ──────────────────

export interface DisplayEvent {
  source_id: string;
  magnitude: number;
  latitude: number;
  longitude: number;
  event_time: string;       // ISO 8601
  region_name?: string;
  depth_km?: number;
  /** True when this record arrived via WebSocket (not from REST API history) */
  is_live: boolean;
}

export function apiEventToDisplay(e: EarthquakeEvent): DisplayEvent {
  return {
    source_id: e.source_id,
    magnitude: e.magnitude,
    latitude: e.latitude,
    longitude: e.longitude,
    event_time: e.event_time,
    region_name: e.region_name ?? undefined,
    depth_km: e.depth_km ?? undefined,
    is_live: false,
  };
}

export function enrichedToDisplay(e: EnrichedEvent): DisplayEvent {
  return {
    source_id: e.source_id,
    magnitude: e.ml_magnitude,
    latitude: e.latitude,
    longitude: e.longitude,
    event_time: new Date(e.event_time_ms).toISOString(),
    region_name: e.region_name ?? undefined,
    depth_km: e.depth_km ?? undefined,
    is_live: true,
  };
}
