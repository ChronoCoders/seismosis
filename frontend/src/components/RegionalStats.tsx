'use client';

import type { BandStats } from '@/types';
import { getMagnitudeInfo } from '@/lib/magnitude';

interface RegionalStatsProps {
  bands: BandStats[];
}

const BAND_LABELS: Record<string, string> = {
  'minor':    'M < 2',
  'light':    'M 2–4',
  'moderate': 'M 4–6',
  'strong':   'M 6–8',
  'major':    'M 8+',
};

function StatCell({ value, label }: { value: number; label: string }) {
  return (
    <div className="flex flex-col items-center">
      <span className="text-lg font-bold font-mono text-text-primary leading-none">
        {value.toLocaleString('tr-TR')}
      </span>
      <span className="text-xs text-text-muted mt-0.5">{label}</span>
    </div>
  );
}

function BandCard({ band }: { band: BandStats }) {
  const repMag = band.min_magnitude + 0.1;
  const info = getMagnitudeInfo(repMag);
  const displayLabel = BAND_LABELS[band.band] ?? band.band;

  return (
    <div className={`rounded border ${info.borderClass} bg-surface p-4`}>
      <div className="flex items-baseline gap-2 mb-3">
        <span className={`text-base font-bold ${info.textClass}`}>{displayLabel}</span>
        <span className={`text-xs uppercase tracking-wider ${info.textClass}`}>
          {info.label}
        </span>
      </div>
      <div className="grid grid-cols-4 gap-2">
        <StatCell value={band.count_1h} label="1 sa" />
        <StatCell value={band.count_24h} label="24 sa" />
        <StatCell value={band.count_7d} label="7 gün" />
        <StatCell value={band.count_30d} label="30 gün" />
      </div>
      {band.max_mag_24h != null && (
        <div className="mt-2 pt-2 border-t border-border/50">
          <span className="text-xs text-text-muted">
            Son 24 sa maks:{' '}
            <span className={`font-mono font-semibold ${info.textClass}`}>
              M {band.max_mag_24h.toFixed(1)}
            </span>
          </span>
        </div>
      )}
    </div>
  );
}

// ── Magnitude distribution bar chart (SVG, no library) ────────────────────────

export function MagnitudeChart({ bands }: RegionalStatsProps) {
  const maxCount = Math.max(...bands.map((b) => b.count_24h), 1);

  return (
    <div className="space-y-2.5">
      {bands.map((band) => {
        const repMag = band.min_magnitude + 0.1;
        const info = getMagnitudeInfo(repMag);
        const pct = Math.max((band.count_24h / maxCount) * 100, 0);
        const displayLabel = BAND_LABELS[band.band] ?? band.band;

        return (
          <div key={band.band} className="flex items-center gap-3">
            <span className={`text-xs font-mono w-14 shrink-0 ${info.textClass}`}>
              {displayLabel}
            </span>
            <div className="flex-1 bg-surface-elevated rounded-sm h-4 overflow-hidden">
              <div
                className="h-full rounded-sm"
                style={{
                  width: `${pct}%`,
                  backgroundColor: info.dotColor,
                  opacity: 0.85,
                }}
              />
            </div>
            <span className="text-xs font-mono text-text-secondary w-8 text-right shrink-0">
              {band.count_24h}
            </span>
          </div>
        );
      })}
    </div>
  );
}

// ── Regional stats grid ───────────────────────────────────────────────────────

export function RegionalStats({ bands }: RegionalStatsProps) {
  return (
    <div className="grid grid-cols-2 lg:grid-cols-4 gap-3">
      {bands.map((band) => (
        <BandCard key={band.band} band={band} />
      ))}
    </div>
  );
}
