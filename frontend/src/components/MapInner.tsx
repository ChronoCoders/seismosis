'use client';

import { useState, useCallback, useMemo, useRef } from 'react';
import { useRouter } from 'next/navigation';
import Map, {
  Source,
  Layer,
  Popup,
  NavigationControl,
  ScaleControl,
  type MapRef,
  type MapLayerMouseEvent,
  type CircleLayer,
  type FillLayer,
  type LineLayer,
} from 'react-map-gl/maplibre';
import 'maplibre-gl/dist/maplibre-gl.css';
import type { FeatureCollection } from 'geojson';
import type { DisplayEvent } from '@/types';
import {
  getMagnitudeInfo,
  formatMagnitude,
  formatRelativeTime,
  formatDepth,
} from '@/lib/magnitude';

// ── Felt radius circle helper ─────────────────────────────────────────────────

/**
 * Approximate a geodesic circle as a GeoJSON polygon.
 * Returns [longitude, latitude] coordinate pairs for the ring.
 */
function makeCirclePolygon(
  centerLon: number,
  centerLat: number,
  radiusKm: number,
  steps = 36,
): number[][] {
  const coords: number[][] = [];
  const latRad = (centerLat * Math.PI) / 180;
  // degrees per km
  const dLat = radiusKm / 111.32;
  const dLon = radiusKm / (111.32 * Math.cos(latRad));
  for (let i = 0; i <= steps; i++) {
    const a = (i / steps) * 2 * Math.PI;
    coords.push([centerLon + dLon * Math.sin(a), centerLat + dLat * Math.cos(a)]);
  }
  return coords;
}

// ── Layer definitions ─────────────────────────────────────────────────────────

const feltFillLayer: FillLayer = {
  id: 'felt-radii-fill',
  type: 'fill',
  source: 'felt-radii',
  paint: {
    'fill-color': ['get', 'color'],
    'fill-opacity': 0.07,
  },
};

const feltOutlineLayer: LineLayer = {
  id: 'felt-radii-outline',
  type: 'line',
  source: 'felt-radii',
  paint: {
    'line-color': ['get', 'color'],
    'line-opacity': 0.3,
    'line-width': 1,
    'line-dasharray': [4, 4],
  },
};

const circleLayer: CircleLayer = {
  id: 'earthquakes',
  type: 'circle',
  source: 'earthquakes',
  paint: {
    'circle-radius': [
      'interpolate', ['linear'], ['get', 'magnitude'],
      1, 4,  3, 7,  5, 12,  7, 18,  9, 24,
    ],
    'circle-color': [
      'case',
      ['>=', ['get', 'magnitude'], 7.0], '#ef4444',
      ['>=', ['get', 'magnitude'], 5.0], '#f97316',
      ['>=', ['get', 'magnitude'], 3.0], '#eab308',
      '#22c55e',
    ],
    'circle-opacity': 0.85,
    'circle-stroke-width': 1,
    'circle-stroke-color': '#0d0f14',
    'circle-stroke-opacity': 0.6,
  },
};

// Live events get a brighter outer ring
const liveRingLayer: CircleLayer = {
  id: 'earthquakes-live-ring',
  type: 'circle',
  source: 'earthquakes',
  filter: ['==', ['get', 'is_live'], true],
  paint: {
    'circle-radius': [
      'interpolate', ['linear'], ['get', 'magnitude'],
      1, 7,  3, 10,  5, 16,  7, 22,  9, 28,
    ],
    'circle-color': 'transparent',
    'circle-stroke-width': 1.5,
    'circle-stroke-color': [
      'case',
      ['>=', ['get', 'magnitude'], 7.0], '#ef4444',
      ['>=', ['get', 'magnitude'], 5.0], '#f97316',
      ['>=', ['get', 'magnitude'], 3.0], '#eab308',
      '#22c55e',
    ],
    'circle-stroke-opacity': 0.5,
  },
};

// ── Component ─────────────────────────────────────────────────────────────────

interface PopupState {
  lng: number;
  lat: number;
  source_id: string;
  magnitude: number;
  ml_magnitude: number | null;
  region_name: string | null;
  event_time: string;
  depth_km: number | null;
  felt_radius_km: number | null;
  is_aftershock: boolean | null;
  mainshock_magnitude: number | null;
  alert_level: string | null;
  is_live: boolean;
}

interface MapInnerProps {
  events: DisplayEvent[];
}

const MAP_STYLE =
  typeof process !== 'undefined'
    ? (process.env.NEXT_PUBLIC_MAP_STYLE ??
        'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json')
    : 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json';

const ALERT_COLORS: Record<string, string> = {
  RED: '#ef4444',
  ORANGE: '#f97316',
  YELLOW: '#eab308',
};

// Turkey bounding box [west, south, east, north] — used for fitBounds on load.
const TURKEY_BBOX: [[number, number], [number, number]] = [
  [25.0, 35.5],  // SW corner
  [44.8, 42.5],  // NE corner
];

export default function MapInner({ events }: MapInnerProps) {
  const router = useRouter();
  const mapRef = useRef<MapRef>(null);
  const [popup, setPopup] = useState<PopupState | null>(null);

  // fitBounds fires after the style is fully loaded, guaranteeing Turkey fills
  // the viewport regardless of the tile style's own default center.
  const onLoad = useCallback(() => {
    mapRef.current?.fitBounds(TURKEY_BBOX, { padding: 20, duration: 0 });
  }, []);

  // GeoJSON for earthquake dots
  const geojson = useMemo<FeatureCollection>(() => ({
    type: 'FeatureCollection',
    features: events.map((e) => ({
      type: 'Feature',
      geometry: { type: 'Point', coordinates: [e.longitude, e.latitude] },
      properties: {
        source_id: e.source_id,
        magnitude: e.ml_magnitude ?? e.magnitude,
        region_name: e.region_name ?? null,
        event_time: e.event_time,
        depth_km: e.depth_km ?? null,
        is_live: e.is_live,
        ml_magnitude: e.ml_magnitude ?? null,
        felt_radius_km: e.estimated_felt_radius_km ?? null,
        is_aftershock: e.is_aftershock ?? null,
        mainshock_magnitude: e.mainshock_magnitude ?? null,
        alert_level: e.alert_level ?? null,
      },
    })),
  }), [events]);

  // GeoJSON for felt-radius circles (only for enriched events with radius data)
  const feltRadiiGeoJson = useMemo<FeatureCollection>(() => ({
    type: 'FeatureCollection',
    features: events
      .filter((e) => e.estimated_felt_radius_km && e.estimated_felt_radius_km > 0)
      .map((e) => ({
        type: 'Feature',
        geometry: {
          type: 'Polygon',
          coordinates: [makeCirclePolygon(e.longitude, e.latitude, e.estimated_felt_radius_km!)],
        },
        properties: {
          color: getMagnitudeInfo(e.ml_magnitude ?? e.magnitude).dotColor,
        },
      })),
  }), [events]);

  const onMouseEnter = useCallback((e: MapLayerMouseEvent) => {
    const feat = e.features?.[0];
    if (!feat || !feat.geometry || feat.geometry.type !== 'Point') return;
    const p = feat.properties!;
    setPopup({
      lng: feat.geometry.coordinates[0] as number,
      lat: feat.geometry.coordinates[1] as number,
      source_id: p.source_id as string,
      magnitude: p.magnitude as number,
      ml_magnitude: p.ml_magnitude as number | null,
      region_name: p.region_name as string | null,
      event_time: p.event_time as string,
      depth_km: p.depth_km as number | null,
      felt_radius_km: p.felt_radius_km as number | null,
      is_aftershock: p.is_aftershock as boolean | null,
      mainshock_magnitude: p.mainshock_magnitude as number | null,
      alert_level: p.alert_level as string | null,
      is_live: p.is_live as boolean,
    });
  }, []);

  const onMouseLeave = useCallback(() => setPopup(null), []);

  const onClick = useCallback((e: MapLayerMouseEvent) => {
    const feat = e.features?.[0];
    if (!feat) return;
    const source_id = feat.properties?.source_id as string | undefined;
    if (source_id) router.push(`/deprem/${encodeURIComponent(source_id)}`);
  }, [router]);

  return (
    <Map
      ref={mapRef}
      initialViewState={{ longitude: 35.0, latitude: 39.0, zoom: 5.5 }}
      style={{ width: '100%', height: '100%' }}
      mapStyle={MAP_STYLE}
      interactiveLayerIds={['earthquakes']}
      onLoad={onLoad}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onClick={onClick}
      cursor={popup ? 'pointer' : 'grab'}
    >
      <NavigationControl position="top-right" showCompass={false} />
      <ScaleControl position="bottom-left" unit="metric" />

      {/* Felt radius circles — rendered below earthquake dots */}
      <Source id="felt-radii" type="geojson" data={feltRadiiGeoJson}>
        <Layer {...feltFillLayer} />
        <Layer {...feltOutlineLayer} />
      </Source>

      {/* Earthquake epicentre dots */}
      <Source id="earthquakes" type="geojson" data={geojson}>
        <Layer {...liveRingLayer} />
        <Layer {...circleLayer} />
      </Source>

      {popup && (
        <Popup
          longitude={popup.lng}
          latitude={popup.lat}
          closeButton={false}
          closeOnClick={false}
          anchor="bottom"
          offset={14}
        >
          <div className="min-w-[180px] space-y-1">
            {/* Magnitude row */}
            <div className="flex items-center gap-2">
              <span
                className="text-xl font-bold font-mono"
                style={{ color: getMagnitudeInfo(popup.magnitude).dotColor }}
              >
                M {formatMagnitude(popup.magnitude)}
              </span>
              <span
                className="text-xs uppercase tracking-wide"
                style={{ color: getMagnitudeInfo(popup.magnitude).dotColor }}
              >
                {getMagnitudeInfo(popup.magnitude).label}
              </span>
              {popup.alert_level && (
                <span
                  className="text-[10px] font-bold px-1.5 rounded border ml-auto"
                  style={{
                    color: ALERT_COLORS[popup.alert_level] ?? '#eab308',
                    borderColor: ALERT_COLORS[popup.alert_level] ?? '#eab308',
                  }}
                >
                  {popup.alert_level}
                </span>
              )}
            </div>

            {/* ML magnitude note */}
            {popup.ml_magnitude !== null &&
              popup.ml_magnitude !== undefined &&
              Math.abs(popup.ml_magnitude - popup.magnitude) >= 0.1 && (
              <p className="text-xs text-text-muted">
                Ham: M {formatMagnitude(popup.ml_magnitude)}
              </p>
            )}

            {/* Location */}
            {popup.region_name && (
              <p className="text-xs text-text-primary">{popup.region_name}</p>
            )}

            {/* Time · depth */}
            <p className="text-xs text-text-secondary">
              {formatRelativeTime(popup.event_time)}
              {popup.depth_km != null && ` · ${formatDepth(popup.depth_km)}`}
            </p>

            {/* Felt radius */}
            {popup.felt_radius_km != null && popup.felt_radius_km > 0 && (
              <p className="text-xs text-text-muted">
                Hissedilme alanı ~{Math.round(popup.felt_radius_km)} km yarıçap
              </p>
            )}

            {/* Aftershock classification */}
            {popup.is_live && popup.is_aftershock !== null && (
              <p className="text-xs font-semibold text-text-secondary">
                {popup.is_aftershock
                  ? `ARTÇI${popup.mainshock_magnitude != null ? ` (Ana şok M ${formatMagnitude(popup.mainshock_magnitude)})` : ''}`
                  : 'ANA ŞOK'}
              </p>
            )}

            <p className="text-xs text-text-muted mt-1">Detay için tıkla →</p>
          </div>
        </Popup>
      )}
    </Map>
  );
}
