'use client';

import Map, {
  Marker,
  NavigationControl,
  ScaleControl,
} from 'react-map-gl/maplibre';
import 'maplibre-gl/dist/maplibre-gl.css';
import { getMagnitudeInfo, formatMagnitude } from '@/lib/magnitude';

interface EpicenterMapProps {
  latitude: number;
  longitude: number;
  magnitude: number;
}

const MAP_STYLE =
  typeof process !== 'undefined'
    ? (process.env.NEXT_PUBLIC_MAP_STYLE ??
        'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json')
    : 'https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json';

export default function EpicenterMap({
  latitude,
  longitude,
  magnitude,
}: EpicenterMapProps) {
  const info = getMagnitudeInfo(magnitude);

  // Scale marker size with magnitude: M2→20px, M5→34px, M8→48px
  const size = Math.round(20 + Math.min(magnitude, 8) * 4);

  return (
    <Map
      initialViewState={{ longitude, latitude, zoom: 8 }}
      style={{ width: '100%', height: '100%' }}
      mapStyle={MAP_STYLE}
      cursor="grab"
    >
      <NavigationControl position="top-right" showCompass={false} />
      <ScaleControl position="bottom-left" unit="metric" />

      <Marker longitude={longitude} latitude={latitude} anchor="center">
        <div
          style={{
            width: size,
            height: size,
            borderRadius: '50%',
            backgroundColor: info.dotColor,
            opacity: 0.85,
            border: '2px solid rgba(255,255,255,0.25)',
            boxShadow: `0 0 0 4px ${info.dotColor}33, 0 0 12px ${info.dotColor}55`,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            cursor: 'default',
          }}
          title={`M ${formatMagnitude(magnitude)}`}
        />
      </Marker>
    </Map>
  );
}
