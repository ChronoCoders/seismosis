'use client';

import { useState, useCallback, useMemo } from 'react';
import { useRouter } from 'next/navigation';
import Map, {
  Source,
  Layer,
  Popup,
  NavigationControl,
  ScaleControl,
  type MapLayerMouseEvent,
  type CircleLayer,
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

// ─── Layer definitions ────────────────────────────────────────────────────────

const circleLayer: CircleLayer = {
  id: 'earthquakes',
  type: 'circle',
  source: 'earthquakes',
  paint: {
    'circle-radius': [
      'interpolate',
      ['linear'],
      ['get', 'magnitude'],
      1, 4,
      3, 7,
      5, 12,
      7, 18,
      9, 24,
    ],
    'circle-color': [
      'case',
      ['>=', ['get', 'magnitude'], 7.0], '#ef4444',
      ['>=', ['get', 'magnitude'], 5.0], '#f97316',
      ['>=', ['get', 'magnitude'], 3.0], '#eab308',
      '#22c55e',
    ],
    'circle-opacity': 0.82,
    'circle-stroke-width': 1,
    'circle-stroke-color': '#0d0f14',
    'circle-stroke-opacity': 0.6,
  },
};

// Live events (from WebSocket) get a brighter ring
const liveRingLayer: CircleLayer = {
  id: 'earthquakes-live-ring',
  type: 'circle',
  source: 'earthquakes',
  filter: ['==', ['get', 'is_live'], true],
  paint: {
    'circle-radius': [
      'interpolate',
      ['linear'],
      ['get', 'magnitude'],
      1, 7,
      3, 10,
      5, 16,
      7, 22,
      9, 28,
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

// ─── Component ────────────────────────────────────────────────────────────────

interface PopupState {
  lng: number;
  lat: number;
  source_id: string;
  magnitude: number;
  region_name: string | null;
  event_time: string;
  depth_km: number | null;
}

interface MapInnerProps {
  events: DisplayEvent[];
}

const MAP_STYLE =
  typeof process !== 'undefined'
    ? (process.env.NEXT_PUBLIC_MAP_STYLE ??
        'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json')
    : 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json';

export default function MapInner({ events }: MapInnerProps) {
  const router = useRouter();
  const [popup, setPopup] = useState<PopupState | null>(null);

  const geojson = useMemo<FeatureCollection>(() => ({
    type: 'FeatureCollection',
    features: events.map((e) => ({
      type: 'Feature',
      geometry: {
        type: 'Point',
        coordinates: [e.longitude, e.latitude],
      },
      properties: {
        source_id: e.source_id,
        magnitude: e.magnitude,
        region_name: e.region_name ?? null,
        event_time: e.event_time,
        depth_km: e.depth_km ?? null,
        is_live: e.is_live,
      },
    })),
  }), [events]);

  const onMouseEnter = useCallback((e: MapLayerMouseEvent) => {
    const feat = e.features?.[0];
    if (!feat || !feat.geometry || feat.geometry.type !== 'Point') return;
    const props = feat.properties!;
    setPopup({
      lng: feat.geometry.coordinates[0] as number,
      lat: feat.geometry.coordinates[1] as number,
      source_id: props.source_id as string,
      magnitude: props.magnitude as number,
      region_name: props.region_name as string | null,
      event_time: props.event_time as string,
      depth_km: props.depth_km as number | null,
    });
  }, []);

  const onMouseLeave = useCallback(() => {
    setPopup(null);
  }, []);

  const onClick = useCallback((e: MapLayerMouseEvent) => {
    const feat = e.features?.[0];
    if (!feat) return;
    const source_id = feat.properties?.source_id as string | undefined;
    if (source_id) {
      router.push(`/deprem/${encodeURIComponent(source_id)}`);
    }
  }, [router]);

  return (
    <Map
      initialViewState={{ longitude: 35.0, latitude: 39.0, zoom: 4.5 }}
      style={{ width: '100%', height: '100%' }}
      mapStyle={MAP_STYLE}
      interactiveLayerIds={['earthquakes']}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      onClick={onClick}
      cursor={popup ? 'pointer' : 'grab'}
      reuseMaps
    >
      <NavigationControl position="top-right" showCompass={false} />
      <ScaleControl position="bottom-left" unit="metric" />

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
          <div className="min-w-[160px]">
            <div className="flex items-center gap-2 mb-1">
              <span
                className="text-lg font-bold font-mono"
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
            </div>
            {popup.region_name && (
              <p className="text-xs text-text-primary mb-0.5">{popup.region_name}</p>
            )}
            <p className="text-xs text-text-secondary">
              {formatRelativeTime(popup.event_time)} &middot; {formatDepth(popup.depth_km)}
            </p>
            <p className="text-xs text-text-muted mt-1">Detay için tıkla →</p>
          </div>
        </Popup>
      )}
    </Map>
  );
}
