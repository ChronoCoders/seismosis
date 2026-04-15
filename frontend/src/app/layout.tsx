import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'Seismosis — Gerçek Zamanlı Deprem İzleme',
  description:
    'Türkiye ve çevresindeki bölgede gerçek zamanlı deprem analizi ve risk değerlendirmesi.',
  icons: {
    icon: [
      { url: '/favicon.svg', type: 'image/svg+xml' },
    ],
  },
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="tr">
      <body className="bg-bg text-text-primary min-h-screen">{children}</body>
    </html>
  );
}
