'use client';

import { useState, useCallback, useMemo, useRef, useEffect } from 'react';
import Map, {
  Source,
  Layer,
  Popup,
  NavigationControl,
  ScaleControl,
  type MapRef,
  type MapLayerMouseEvent,
  type CircleLayer,
} from 'react-map-gl/maplibre';
import 'maplibre-gl/dist/maplibre-gl.css';
import type { FeatureCollection } from 'geojson';
import type { EarthquakeEvent, AftershockForecast } from '@/types';
import { formatMagnitude } from '@/lib/magnitude';

const MAP_STYLE =
  typeof process !== 'undefined'
    ? (process.env.NEXT_PUBLIC_MAP_STYLE ??
        'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json')
    : 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json';

const TURKEY_BBOX: [[number, number], [number, number]] = [
  [25.0, 35.5],
  [44.8, 42.5],
];

// Circle layer colored by p_at_least_one: grey → orange → red
const eventLayer: CircleLayer = {
  id: 'aftershock-events',
  type: 'circle',
  source: 'aftershock-events',
  paint: {
    'circle-radius': [
      'interpolate', ['linear'], ['get', 'magnitude'],
      3, 8,  5, 14,  7, 20,
    ],
    'circle-color': [
      'case',
      ['>', ['get', 'p_one'], 0.6], '#ef4444',
      ['>', ['get', 'p_one'], 0.3], '#f97316',
      ['>', ['get', 'p_one'], 0.1], '#eab308',
      '#6b7280',
    ],
    'circle-opacity': 0.82,
    'circle-stroke-width': 1.5,
    'circle-stroke-color': '#0d0f14',
    'circle-stroke-opacity': 0.5,
  },
};

interface PopupInfo {
  lng: number;
  lat: number;
  source_id: string;
  magnitude: number;
  region_name: string | null;
  p_one: number | null;
  expected_count: number | null;
  horizon_days: number | null;
  model_version: string | null;
}

interface Props {
  events: EarthquakeEvent[];
  forecasts: Map<string, AftershockForecast>;
  focusSourceId?: string | null;
}

export default function AftershockMapInner({ events, forecasts, focusSourceId }: Props) {
  const mapRef = useRef<MapRef>(null);
  const [popup, setPopup] = useState<PopupInfo | null>(null);

  useEffect(() => {
    if (!focusSourceId) return;
    const ev = events.find((e) => e.source_id === focusSourceId);
    if (!ev) return;
    mapRef.current?.flyTo({ center: [ev.longitude, ev.latitude], zoom: 8, duration: 800 });
  }, [focusSourceId, events]);

  const onLoad = useCallback(() => {
    mapRef.current?.fitBounds(TURKEY_BBOX, { padding: 20, duration: 0 });
  }, []);

  const geojson = useMemo<FeatureCollection>(() => ({
    type: 'FeatureCollection',
    features: events.map((e) => {
      const f = forecasts.get(e.source_id);
      return {
        type: 'Feature',
        geometry: { type: 'Point', coordinates: [e.longitude, e.latitude] },
        properties: {
          source_id: e.source_id,
          magnitude: e.magnitude,
          region_name: e.region_name ?? null,
          p_one: f?.p_at_least_one ?? -1,
          expected_count: f?.expected_count ?? null,
          horizon_days: f?.horizon_days ?? null,
          model_version: f?.model_version ?? null,
        },
      };
    }),
  }), [events, forecasts]);

  const onMouseEnter = useCallback((e: MapLayerMouseEvent) => {
    const feat = e.features?.[0];
    if (!feat || feat.geometry.type !== 'Point') return;
    const p = feat.properties!;
    setPopup({
      lng: (feat.geometry as GeoJSON.Point).coordinates[0] as number,
      lat: (feat.geometry as GeoJSON.Point).coordinates[1] as number,
      source_id: p.source_id as string,
      magnitude: p.magnitude as number,
      region_name: p.region_name as string | null,
      p_one: (p.p_one as number) >= 0 ? (p.p_one as number) : null,
      expected_count: p.expected_count as number | null,
      horizon_days: p.horizon_days as number | null,
      model_version: p.model_version as string | null,
    });
  }, []);

  const onMouseLeave = useCallback(() => setPopup(null), []);

  return (
    <Map
      ref={mapRef}
      initialViewState={{ longitude: 35.0, latitude: 39.0, zoom: 5 }}
      style={{ width: '100%', height: '100%' }}
      mapStyle={MAP_STYLE}
      interactiveLayerIds={['aftershock-events']}
      onLoad={onLoad}
      onMouseEnter={onMouseEnter}
      onMouseLeave={onMouseLeave}
      cursor={popup ? 'pointer' : 'grab'}
    >
      <NavigationControl position="top-right" showCompass={false} />
      <ScaleControl position="bottom-left" unit="metric" />

      <Source id="aftershock-events" type="geojson" data={geojson}>
        <Layer {...eventLayer} />
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
          <div className="min-w-[160px] space-y-1">
            <p className="text-sm font-bold font-mono text-[#e2e4ed]">
              M {formatMagnitude(popup.magnitude)}
            </p>
            {popup.region_name && (
              <p className="text-xs text-[#8b90a2] truncate max-w-[200px]">
                {popup.region_name}
              </p>
            )}
            {popup.p_one !== null ? (
              <div className="space-y-0.5 pt-1 border-t border-[#232736]">
                <p className="text-xs text-[#575c6e]">
                  P(artçı≥1):{' '}
                  <span className="text-[#e2e4ed] font-mono font-semibold">
                    {(popup.p_one * 100).toFixed(1)}%
                  </span>
                </p>
                {popup.expected_count !== null && (
                  <p className="text-xs text-[#575c6e]">
                    Beklenen artçı:{' '}
                    <span className="text-[#e2e4ed] font-mono">
                      {popup.expected_count.toFixed(2)}
                    </span>
                  </p>
                )}
                {popup.horizon_days !== null && (
                  <p className="text-[10px] text-[#575c6e]">
                    {popup.horizon_days} gün ufuk
                  </p>
                )}
              </div>
            ) : (
              <p className="text-[10px] text-[#575c6e] pt-1">Tahmin hesaplanmadı</p>
            )}
          </div>
        </Popup>
      )}
    </Map>
  );
}
