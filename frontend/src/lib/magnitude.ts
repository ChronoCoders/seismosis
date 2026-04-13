export interface MagnitudeInfo {
  label: string;
  dotColor: string;    // hex, used for SVG/canvas
  textClass: string;   // Tailwind text colour class
  bgClass: string;     // Tailwind background class (muted)
  borderClass: string; // Tailwind border class
}

export function getMagnitudeInfo(magnitude: number): MagnitudeInfo {
  if (magnitude >= 7.0) {
    return {
      label: 'Şiddetli',
      dotColor: '#ef4444',
      textClass: 'text-mag-red',
      bgClass: 'bg-red-950/60',
      borderClass: 'border-mag-red/40',
    };
  }
  if (magnitude >= 5.0) {
    return {
      label: 'Kuvvetli',
      dotColor: '#f97316',
      textClass: 'text-mag-orange',
      bgClass: 'bg-orange-950/60',
      borderClass: 'border-mag-orange/40',
    };
  }
  if (magnitude >= 3.0) {
    return {
      label: 'Orta',
      dotColor: '#eab308',
      textClass: 'text-mag-yellow',
      bgClass: 'bg-yellow-950/60',
      borderClass: 'border-mag-yellow/40',
    };
  }
  return {
    label: 'Hafif',
    dotColor: '#22c55e',
    textClass: 'text-mag-green',
    bgClass: 'bg-green-950/60',
    borderClass: 'border-mag-green/40',
  };
}

export function formatMagnitude(mag: number): string {
  return mag.toFixed(1);
}

export function formatDepth(depth: number | undefined | null): string {
  if (depth == null) return 'Bilinmiyor';
  return `${depth.toFixed(0)} km`;
}

export function formatRelativeTime(isoString: string): string {
  const ms = Date.now() - new Date(isoString).getTime();
  const m = Math.floor(ms / 60_000);
  const h = Math.floor(ms / 3_600_000);
  const d = Math.floor(ms / 86_400_000);
  if (m < 1) return 'Şimdi';
  if (m < 60) return `${m} dk`;
  if (h < 24) return `${h} sa`;
  return `${d} g`;
}

export function formatDateTime(isoString: string): string {
  return new Date(isoString).toLocaleString('tr-TR', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    timeZone: 'Europe/Istanbul',
  });
}

export function formatDateTimeMs(ms: number): string {
  return formatDateTime(new Date(ms).toISOString());
}
