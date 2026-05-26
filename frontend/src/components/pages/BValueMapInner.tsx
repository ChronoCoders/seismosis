'use client';

import { useState, useCallback, useRef } from 'react';
import Map, {
  Source,
  Layer,
  Popup,
  NavigationControl,
  ScaleControl,
  type MapRef,
  type MapLayerMouseEvent,
  type FillLayer,
  type LineLayer,
} from 'react-map-gl/maplibre';
import 'maplibre-gl/dist/maplibre-gl.css';
import type { GrMapResponse } from '@/types';

const MAP_STYLE =
  typeof process !== 'undefined'
    ? (process.env.NEXT_PUBLIC_MAP_STYLE ??
        'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json')
    : 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json';

const TURKEY_BBOX: [[number, number], [number, number]] = [
  [25.0, 35.5],
  [44.8, 42.5],
];

// Blue (low b) → red (high b) choropleth
const cellFillLayer: FillLayer = {
  id: 'bvalue-fill',
  type: 'fill',
  source: 'bvalue-cells',
  paint: {
    'fill-color': [
      'interpolate', ['linear'], ['get', 'b_value'],
      0.5, '#3b82f6',   // blue — low b (many large events)
      0.8, '#14b8a6',   // teal
      1.0, '#eab308',   // yellow — typical b~1
      1.2, '#f97316',   // orange
      1.5, '#ef4444',   // red — high b (many small events)
    ],
    'fill-opacity': 0.65,
  },
};

const cellOutlineLayer: LineLayer = {
  id: 'bvalue-outline',
  type: 'line',
  source: 'bvalue-cells',
  paint: {
    'line-color': '#0d0f14',
    'line-width': 0.5,
    'line-opacity': 0.6,
  },
};

interface PopupInfo {
  lng: number;
  lat: number;
  b_value: number;
  b_std: number;
  n_events: number;
  region_name: string | null;
}

interface Props {
  grMap: GrMapResponse | null;
}

export default function BValueMapInner({ grMap }: Props) {
  const mapRef = useRef<MapRef>(null);
  const [popup, setPopup] = useState<PopupInfo | null>(null);

  const onLoad = useCallback(() => {
    mapRef.current?.fitBounds(TURKEY_BBOX, { padding: 20, duration: 0 });
  }, []);

  const geojson = {
    type: 'FeatureCollection' as const,
    features: grMap?.features ?? [],
  };

  const onMouseEnter = useCallback((e: MapLayerMouseEvent) => {
    const feat = e.features?.[0];
    if (!feat) return;
    const p = feat.properties!;
    // Use centroid of clicked point for popup position
    setPopup({
      lng: e.lngLat.lng,
      lat: e.lngLat.lat,
      b_value: p.b_value as number,
      b_std: p.b_std as number,
      n_events: p.n_events as number,
      region_name: p.region_name as string | null,
    });
  }, []);

  const onMouseLeave = useCallback(() => setPopup(null), []);

  const hasData = (grMap?.features.length ?? 0) > 0;

  return (
    <div className="relative w-full h-full">
      <Map
        ref={mapRef}
        initialViewState={{ longitude: 35.0, latitude: 39.0, zoom: 5 }}
        style={{ width: '100%', height: '100%' }}
        mapStyle={MAP_STYLE}
        interactiveLayerIds={hasData ? ['bvalue-fill'] : []}
        onLoad={onLoad}
        onMouseEnter={hasData ? onMouseEnter : undefined}
        onMouseLeave={hasData ? onMouseLeave : undefined}
        cursor={popup ? 'crosshair' : 'grab'}
      >
        <NavigationControl position="top-right" showCompass={false} />
        <ScaleControl position="bottom-left" unit="metric" />

        <Source id="bvalue-cells" type="geojson" data={geojson}>
          <Layer {...cellFillLayer} />
          <Layer {...cellOutlineLayer} />
        </Source>

        {popup && (
          <Popup
            longitude={popup.lng}
            latitude={popup.lat}
            closeButton={false}
            closeOnClick={false}
            anchor="bottom"
            offset={8}
          >
            <div className="min-w-[140px] space-y-1">
              <p className="text-xs font-bold text-[#e2e4ed] font-mono">
                b = {popup.b_value.toFixed(3)} ± {popup.b_std.toFixed(3)}
              </p>
              {popup.region_name && (
                <p className="text-[10px] text-[#8b90a2] truncate max-w-[180px]">
                  {popup.region_name}
                </p>
              )}
              <p className="text-[10px] text-[#575c6e]">
                {popup.n_events.toLocaleString()} olay kullanıldı
              </p>
            </div>
          </Popup>
        )}
      </Map>

      {/* Empty state overlay */}
      {!hasData && (
        <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
          <div className="bg-[#0c0e16]/80 rounded-lg px-4 py-3 text-center border border-[#232736]">
            <p className="text-xs font-semibold text-[#8b90a2]">b-değeri hesaplanmadı</p>
            <p className="text-[10px] text-[#575c6e] mt-1">
              Tahmin servisi çalıştırıldıktan sonra görünür
            </p>
          </div>
        </div>
      )}

      {/* B-value legend */}
      {hasData && (
        <div className="absolute bottom-8 right-2 bg-[#0c0e16]/80 rounded px-2 py-1.5 border border-[#232736] text-[9px] space-y-0.5">
          {[
            { color: '#3b82f6', label: '< 0.7' },
            { color: '#14b8a6', label: '0.7–1.0' },
            { color: '#eab308', label: '1.0–1.2' },
            { color: '#f97316', label: '1.2–1.5' },
            { color: '#ef4444', label: '> 1.5' },
          ].map(({ color, label }) => (
            <div key={label} className="flex items-center gap-1.5">
              <span className="w-3 h-2 rounded-sm shrink-0" style={{ backgroundColor: color }} />
              <span className="text-[#8b90a2] font-mono">{label}</span>
            </div>
          ))}
          <p className="text-[#575c6e] mt-0.5 font-bold">b-değeri</p>
        </div>
      )}
    </div>
  );
}
