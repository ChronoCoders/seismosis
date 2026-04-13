import { notFound } from 'next/navigation';
import Link from 'next/link';
import dynamic from 'next/dynamic';
import type { Metadata } from 'next';
import { fetchEvent } from '@/lib/api';
import { MagnitudeBadge } from '@/components/MagnitudeBadge';
import { getMagnitudeInfo, formatDateTime, formatDepth } from '@/lib/magnitude';
import type { DisplayEvent } from '@/types';

interface Props {
  params: { id: string };
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const event = await fetchEvent(params.id).catch(() => null);
  if (!event) return { title: 'Deprem Bulunamadı — Seismosis' };
  return {
    title: `M ${event.magnitude.toFixed(1)} — ${event.region_name ?? params.id} | Seismosis`,
  };
}

// ─── Mini map (client-only) ───────────────────────────────────────────────────

const DetailMap = dynamic(() => import('@/components/MapInner'), {
  ssr: false,
  loading: () => (
    <div className="w-full h-full bg-surface flex items-center justify-center text-text-muted text-sm">
      Harita yükleniyor…
    </div>
  ),
});

// ─── Table row helper ─────────────────────────────────────────────────────────

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <tr className="border-b border-border last:border-0">
      <th className="py-2.5 px-4 text-left text-sm text-text-secondary font-normal whitespace-nowrap w-48">
        {label}
      </th>
      <td className="py-2.5 px-4 text-sm text-text-primary font-mono">{value}</td>
    </tr>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export default async function DepremDetayPage({ params }: Props) {
  const event = await fetchEvent(params.id).catch(() => null);
  if (!event) notFound();

  const info = getMagnitudeInfo(event.magnitude);

  const mapEvent: DisplayEvent = {
    source_id: event.source_id,
    magnitude: event.magnitude,
    latitude: event.latitude,
    longitude: event.longitude,
    event_time: event.event_time,
    region_name: event.region_name ?? undefined,
    depth_km: event.depth_km ?? undefined,
    is_live: false,
  };

  return (
    <div className="min-h-screen bg-bg">
      {/* Header */}
      <header className="h-14 bg-surface border-b border-border flex items-center px-4 gap-4">
        <Link
          href="/"
          className="flex items-center gap-1.5 text-text-secondary hover:text-text-primary text-sm transition-colors"
        >
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
            <path
              d="M9 3L5 7l4 4"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          Geri
        </Link>
        <span className="text-border">|</span>
        <span className="text-text-muted text-sm font-mono">{event.source_id}</span>
      </header>

      <main className="max-w-5xl mx-auto px-4 py-6 space-y-6">
        {/* Top grid: info card + map */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {/* ── Info card ── */}
          <div className={`rounded border ${info.borderClass} bg-surface p-6`}>
            <div className="flex items-start gap-4 mb-6">
              <MagnitudeBadge magnitude={event.magnitude} size="xl" showLabel />
              <div>
                <h1 className="text-text-primary text-xl font-semibold leading-tight">
                  {event.region_name ?? 'Konum bilinmiyor'}
                </h1>
                <p className="text-text-secondary text-sm mt-1">
                  {event.latitude.toFixed(4)}°N, {event.longitude.toFixed(4)}°E
                </p>
              </div>
            </div>

            <dl className="space-y-2">
              <div className="flex justify-between">
                <dt className="text-sm text-text-secondary">Derinlik</dt>
                <dd className="text-sm font-mono text-text-primary">
                  {formatDepth(event.depth_km)}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-sm text-text-secondary">Olay zamanı</dt>
                <dd className="text-sm font-mono text-text-primary">
                  {formatDateTime(event.event_time)}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-sm text-text-secondary">Büyüklük ölçeği</dt>
                <dd className="text-sm font-mono text-text-primary">
                  {event.magnitude_type}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-sm text-text-secondary">Kaynak ağ</dt>
                <dd className="text-sm font-mono text-text-primary uppercase">
                  {event.source_network}
                </dd>
              </div>
              <div className="flex justify-between">
                <dt className="text-sm text-text-secondary">Veri kalitesi</dt>
                <dd className="text-sm font-mono text-text-primary">
                  {event.quality_indicator}
                </dd>
              </div>
            </dl>
          </div>

          {/* ── Mini map ── */}
          <div className="rounded border border-border overflow-hidden min-h-[280px] md:min-h-0">
            <DetailMap events={[mapEvent]} />
          </div>
        </div>

        {/* Technical details table */}
        <section>
          <h2 className="text-xs font-semibold uppercase tracking-widest text-text-muted mb-3">
            Teknik Detaylar
          </h2>
          <div className="rounded border border-border bg-surface overflow-hidden">
            <table className="w-full">
              <tbody>
                <Row label="Kaynak Kimliği" value={event.source_id} />
                <Row label="Veritabanı UUID" value={event.id} />
                <Row label="İşlenme Zamanı" value={formatDateTime(event.processed_at)} />
                <Row label="Boru Hattı Sürümü" value={event.pipeline_version} />
                <Row
                  label="Koordinatlar"
                  value={`${event.latitude.toFixed(6)}, ${event.longitude.toFixed(6)}`}
                />
                <Row
                  label="Derinlik"
                  value={formatDepth(event.depth_km)}
                />
                <Row label="Ham Büyüklük" value={`${event.magnitude.toFixed(2)} ${event.magnitude_type}`} />
                <Row label="Veri Kalitesi" value={event.quality_indicator} />
              </tbody>
            </table>
          </div>
        </section>
      </main>
    </div>
  );
}
