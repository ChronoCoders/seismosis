'use client';

import { useState, useEffect } from 'react';
import { LineChart, Activity, AlertTriangle, TrendingDown } from 'lucide-react';
import type { EarthquakeEvent, AftershockForecast, GrAnalysis, GrMapResponse } from '@/types';
import {
  fetchEvents,
  fetchAftershockForecast,
  fetchRegionalForecast,
  fetchGrAnalysis,
  fetchGrMap,
} from '@/lib/api';
import { formatMagnitude, formatRelativeTime } from '@/lib/magnitude';
import AftershockMapInner from './AftershockMapInner';
import BValueMapInner from './BValueMapInner';

// ── Probability badge colour ──────────────────────────────────────────────────

function pColor(p: number): string {
  if (p > 0.6) return 'text-red-400';
  if (p > 0.3) return 'text-orange-400';
  if (p > 0.1) return 'text-yellow-400';
  return 'text-[#6b7280]';
}

function pLabel(p: number): string {
  if (p > 0.6) return 'Yüksek';
  if (p > 0.3) return 'Orta';
  if (p > 0.1) return 'Düşük';
  return 'Çok Düşük';
}

// ── Panel shell ───────────────────────────────────────────────────────────────

function Panel({
  title,
  subtitle,
  icon: Icon,
  children,
}: {
  title: string;
  subtitle: string;
  icon: React.ElementType;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col min-h-0 rounded-lg border border-[#232736] bg-[#0c0e16] overflow-hidden">
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-[#232736] bg-[#12151d] shrink-0">
        <Icon size={13} className="text-[#4a90e2] shrink-0" />
        <div className="min-w-0">
          <p className="text-xs font-semibold text-[#e2e4ed] leading-none">{title}</p>
          <p className="text-[9px] text-[#575c6e] mt-0.5 leading-none">{subtitle}</p>
        </div>
      </div>
      <div className="flex-1 min-h-0">{children}</div>
    </div>
  );
}

// ── RecentForecastsList ───────────────────────────────────────────────────────

function RecentForecastsList({
  events,
  forecasts,
  selectedSourceId,
  onSelect,
}: {
  events: EarthquakeEvent[];
  forecasts: Map<string, AftershockForecast>;
  selectedSourceId: string | null;
  onSelect: (sourceId: string) => void;
}) {
  const withForecasts = events
    .map((e) => ({ event: e, forecast: forecasts.get(e.source_id) }))
    .filter((x) => x.forecast !== undefined);

  if (withForecasts.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full gap-3 p-4 opacity-60">
        <TrendingDown size={36} strokeWidth={1} className="text-[#575c6e]" />
        <div className="text-center">
          <p className="text-xs font-medium text-[#8b90a2]">Tahmin verisi yok</p>
          <p className="text-[10px] text-[#575c6e] mt-1">
            ETAS modeli M≥4.0 olaylardan sonra çalışır
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="overflow-y-auto h-full p-3 space-y-2">
      {withForecasts.map(({ event: ev, forecast: f }) => (
        <div
          key={ev.source_id}
          onClick={() => onSelect(ev.source_id)}
          className={`rounded border bg-[#12151d] px-3 py-2.5 flex items-start gap-3 cursor-pointer transition-colors ${
            selectedSourceId === ev.source_id
              ? 'border-[#4a90e2] ring-1 ring-[#4a90e2]/30'
              : 'border-[#232736] hover:border-[#3a4055]'
          }`}
        >
          {/* Magnitude */}
          <div className="shrink-0 w-12 text-center pt-0.5">
            <p className="text-base font-bold font-mono text-[#e2e4ed] leading-none">
              M {formatMagnitude(ev.magnitude)}
            </p>
            <p className="text-[9px] text-[#575c6e] mt-0.5">{ev.magnitude_type}</p>
          </div>

          {/* Info */}
          <div className="flex-1 min-w-0">
            <p className="text-[11px] font-semibold text-[#e2e4ed] truncate">
              {ev.region_name ?? `${ev.latitude.toFixed(2)}°N ${ev.longitude.toFixed(2)}°E`}
            </p>
            <p className="text-[10px] font-mono text-[#575c6e] mt-0.5">
              {formatRelativeTime(ev.event_time)}
            </p>
          </div>

          {/* Forecast probability */}
          <div className="shrink-0 text-right">
            <p className={`text-sm font-bold font-mono ${pColor(f!.p_at_least_one)}`}>
              {(f!.p_at_least_one * 100).toFixed(0)}%
            </p>
            <p className={`text-[9px] font-semibold ${pColor(f!.p_at_least_one)}`}>
              {pLabel(f!.p_at_least_one)}
            </p>
            <p className="text-[9px] text-[#575c6e] mt-0.5 font-mono">
              {f!.horizon_days}g ufuk
            </p>
          </div>
        </div>
      ))}
    </div>
  );
}

// ── ModelConfidencePanel ──────────────────────────────────────────────────────

const CATALOG_STALE_DAYS = 7;

function ModelConfidencePanel({ gr }: { gr: GrAnalysis | null }) {
  const isRuleBased = !gr;

  const isCatalogStale = gr
    ? (Date.now() - new Date(gr.catalog_end).getTime()) / 86_400_000 > CATALOG_STALE_DAYS
    : false;

  return (
    <div className="overflow-y-auto h-full p-4 space-y-4">
      {/* Catalog staleness warning */}
      {isCatalogStale && (
        <div className="flex items-start gap-2 rounded border border-orange-700/40 bg-orange-950/20 px-3 py-2">
          <AlertTriangle size={12} className="text-orange-400 shrink-0 mt-0.5" />
          <p className="text-[10px] text-orange-300">
            Katalog güncel değil — son analiz {Math.floor((Date.now() - new Date(gr!.catalog_end).getTime()) / 86_400_000)} gün önce
          </p>
        </div>
      )}

      {/* GR Analysis block */}
      <div>
        <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-2">
          Gutenberg-Richter Analizi
        </p>
        {gr ? (
          <div className="grid grid-cols-2 gap-x-4 gap-y-2">
            {[
              { label: 'b-değeri', value: `${gr.b_value.toFixed(3)} ± ${gr.b_std.toFixed(3)}` },
              { label: 'a-değeri', value: gr.a_value.toFixed(3) },
              { label: 'Mc', value: gr.mc.toFixed(2) },
              { label: 'Olay sayısı', value: gr.n_events.toLocaleString() },
            ].map(({ label, value }) => (
              <div key={label} className="rounded border border-[#232736] bg-[#12151d] px-3 py-2">
                <p className="text-[9px] text-[#575c6e] leading-none">{label}</p>
                <p className="text-sm font-bold font-mono text-[#e2e4ed] mt-1 leading-none">
                  {value}
                </p>
              </div>
            ))}
            <div className="col-span-2 text-[9px] text-[#575c6e] font-mono">
              {new Date(gr.catalog_start).toLocaleDateString('tr-TR')} –{' '}
              {new Date(gr.catalog_end).toLocaleDateString('tr-TR')} · {gr.model_version}
            </div>
          </div>
        ) : (
          <div className="rounded border border-[#232736] bg-[#12151d] px-3 py-3 text-center">
            <p className="text-xs text-[#575c6e]">Katalog analizi bekleniyor</p>
          </div>
        )}
      </div>

      {/* Classifier block */}
      <div>
        <p className="text-[9px] font-bold uppercase tracking-widest text-[#575c6e] mb-2">
          Sismik Sınıflandırıcı
        </p>
        {isRuleBased && (
          <div className="flex items-start gap-2 rounded border border-yellow-700/40 bg-yellow-950/20 px-3 py-2 mb-2">
            <AlertTriangle size={12} className="text-yellow-500 shrink-0 mt-0.5" />
            <p className="text-[10px] text-yellow-400">
              Eğitilmiş model yok — kural tabanlı sınıflandırma aktif
            </p>
          </div>
        )}
        <div className="grid grid-cols-2 gap-x-4 gap-y-2">
          {[
            { label: 'Model türü', value: isRuleBased ? 'Kural tabanlı' : 'HistGradBoost' },
            { label: 'Sınıflar', value: 'Tektonik / İndüklenmiş / Volkanik' },
            { label: 'Durum', value: isRuleBased ? 'Yedek mod' : 'Aktif' },
            { label: 'Makro-F1', value: isRuleBased ? '—' : '≥ 0.75 hedef' },
          ].map(({ label, value }) => (
            <div key={label} className="rounded border border-[#232736] bg-[#12151d] px-3 py-2">
              <p className="text-[9px] text-[#575c6e] leading-none">{label}</p>
              <p className="text-[10px] font-semibold text-[#8b90a2] mt-1 leading-none break-words">
                {value}
              </p>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ── ForecastPage ──────────────────────────────────────────────────────────────

const PERIOD_OPTIONS = [
  { label: '7 gün',  value: 7  },
  { label: '30 gün', value: 30 },
  { label: '90 gün', value: 90 },
] as const;

const MAG_OPTIONS = [
  { label: 'M≥2.0', value: 2.0 },
  { label: 'M≥3.0', value: 3.0 },
  { label: 'M≥4.0', value: 4.0 },
] as const;

export function ForecastPage() {
  const [loading, setLoading]               = useState(true);
  const [events, setEvents]                 = useState<EarthquakeEvent[]>([]);
  const [forecasts, setForecasts]           = useState<Map<string, AftershockForecast>>(new Map());
  const [grAnalysis, setGrAnalysis]         = useState<GrAnalysis | null>(null);
  const [grMap, setGrMap]                   = useState<GrMapResponse | null>(null);
  const [period, setPeriod]                 = useState<number>(30);
  const [minMag, setMinMag]                 = useState<number>(4.0);
  const [selectedSourceId, setSelectedSourceId] = useState<string | null>(null);

  useEffect(() => {
    async function load() {
      setLoading(true);
      try {
        // Parallel: M≥minMag events, GR analysis, GR map, regional forecast
        const [evResp, grA, grM] = await Promise.allSettled([
          fetchEvents({ min_magnitude: minMag, page_size: 10 }),
          fetchGrAnalysis(),
          fetchGrMap({ min_magnitude: minMag }),
          fetchRegionalForecast({ horizon_days: period, min_magnitude: minMag }).catch(() => null),
        ]);

        const evList = evResp.status === 'fulfilled' ? evResp.value.events : [];
        setEvents(evList);
        if (grA.status === 'fulfilled') setGrAnalysis(grA.value);
        if (grM.status === 'fulfilled') setGrMap(grM.value);

        // Fetch ETAS forecasts for each event in parallel
        if (evList.length > 0) {
          const results = await Promise.allSettled(
            evList.map(async (e) => {
              const f = await fetchAftershockForecast(e.source_id);
              return { source_id: e.source_id, forecast: f };
            }),
          );
          const fm = new Map<string, AftershockForecast>();
          for (const r of results) {
            if (r.status === 'fulfilled' && r.value.forecast) {
              fm.set(r.value.source_id, r.value.forecast);
            }
          }
          setForecasts(fm);
        }
      } finally {
        setLoading(false);
      }
    }
    void load();
  }, [period, minMag]);

  return (
    <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-5 h-12 border-b border-[#232736] bg-[#12151d] shrink-0">
        <div className="flex items-center gap-2">
          <LineChart size={15} className="text-[#4a90e2]" />
          <div>
            <h1 className="text-sm font-semibold text-[#e2e4ed]">Tahmin</h1>
            <p className="text-[10px] text-[#575c6e] mt-0.5">
              ETAS artçı tahmini · Gutenberg-Richter analizi · Sismik sınıflandırma
            </p>
          </div>
        </div>

        {/* Filter controls */}
        <div className="flex items-center gap-3">
          {/* Period selector */}
          <div className="flex items-center gap-1">
            {PERIOD_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setPeriod(opt.value)}
                className={`px-2 py-1 rounded text-[10px] font-semibold transition-colors ${
                  period === opt.value
                    ? 'bg-[#4a90e2] text-white'
                    : 'bg-[#232736] text-[#8b90a2] hover:bg-[#2c3044] hover:text-[#e2e4ed]'
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>

          {/* Magnitude selector */}
          <div className="flex items-center gap-1">
            {MAG_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setMinMag(opt.value)}
                className={`px-2 py-1 rounded text-[10px] font-semibold transition-colors ${
                  minMag === opt.value
                    ? 'bg-[#4a90e2] text-white'
                    : 'bg-[#232736] text-[#8b90a2] hover:bg-[#2c3044] hover:text-[#e2e4ed]'
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>

          {loading && (
            <span className="text-[10px] font-mono text-[#575c6e] animate-pulse">Yükleniyor…</span>
          )}
        </div>
      </div>

      {/* 2×2 grid */}
      <div className="flex-1 min-h-0 grid grid-cols-2 grid-rows-2 gap-3 p-4 overflow-hidden">

        {/* Top-left: Aftershock Heatmap */}
        <Panel
          title="Artçı Sarsıntı Olasılık Haritası"
          subtitle="M≥4.0 olaylar · artçı olasılığına göre renklendirme"
          icon={Activity}
        >
          <AftershockMapInner events={events} forecasts={forecasts} focusSourceId={selectedSourceId} />
        </Panel>

        {/* Top-right: B-Value Map */}
        <Panel
          title="b-Değeri Haritası"
          subtitle="Gutenberg-Richter uzamsal analizi · mavi=düşük · kırmızı=yüksek"
          icon={TrendingDown}
        >
          <BValueMapInner grMap={grMap} />
        </Panel>

        {/* Bottom-left: Recent Forecasts List */}
        <Panel
          title="Son Artçı Sarsıntı Tahminleri"
          subtitle="M≥4.0 son olaylar için ETAS çıktısı"
          icon={LineChart}
        >
          <RecentForecastsList
            events={events}
            forecasts={forecasts}
            selectedSourceId={selectedSourceId}
            onSelect={setSelectedSourceId}
          />
        </Panel>

        {/* Bottom-right: Model Confidence */}
        <Panel
          title="Model Güven Paneli"
          subtitle="Sınıflandırıcı doğruluk · katalog bilgisi · model sürümleri"
          icon={Activity}
        >
          <ModelConfidencePanel gr={grAnalysis} />
        </Panel>

      </div>
    </div>
  );
}
