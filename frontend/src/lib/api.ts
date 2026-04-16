import type { EarthquakeEvent, EventListResponse, StatsResponse } from '@/types';

/**
 * Returns the API base URL.
 * - Client-side: uses Next.js rewrite proxy (/api/v1) to avoid CORS.
 * - Server-side: uses the internal Docker network URL directly.
 */
function apiBase(): string {
  if (typeof window === 'undefined') {
    return `${process.env.API_URL ?? 'http://seismosis-api:8000'}/v1`;
  }
  return '/api/v1';
}

export async function fetchEvents(
  params: Record<string, string | number> = {},
): Promise<EventListResponse> {
  const url = new URL(`${apiBase()}/events`, 'http://placeholder');
  for (const [k, v] of Object.entries(params)) {
    url.searchParams.set(k, String(v));
  }
  const res = await fetch(url.pathname + url.search, { next: { revalidate: 0 } });
  if (!res.ok) throw new Error(`GET /v1/events → HTTP ${res.status}`);
  return res.json() as Promise<EventListResponse>;
}

export async function fetchEvent(sourceId: string): Promise<EarthquakeEvent | null> {
  const res = await fetch(
    `${apiBase()}/events/${encodeURIComponent(sourceId)}`,
    { next: { revalidate: 60 } },
  );
  if (res.status === 404) return null;
  if (!res.ok) throw new Error(`GET /v1/events/${sourceId} → HTTP ${res.status}`);
  return res.json() as Promise<EarthquakeEvent>;
}

export async function fetchStats(): Promise<StatsResponse> {
  const res = await fetch(`${apiBase()}/stats`, { cache: 'no-store' });
  if (!res.ok) throw new Error(`GET /v1/stats → HTTP ${res.status}`);
  return res.json() as Promise<StatsResponse>;
}
