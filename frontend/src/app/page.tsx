import { Dashboard } from '@/components/Dashboard';

// Ana Sayfa — delegated entirely to the Dashboard client component.
// Keeping this as a server component means the route still participates in
// Next.js page caching; the heavy data fetching and WebSocket work is
// contained inside Dashboard.
export default function HomePage() {
  return <Dashboard />;
}
