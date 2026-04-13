import type { NextConfig } from 'next';

const nextConfig: NextConfig = {
  output: 'standalone',

  // Proxy /api/v1/* to the backend API service so the browser never
  // makes cross-origin requests and no CORS headers are needed.
  async rewrites() {
    const apiUrl = process.env.API_URL ?? 'http://seismosis-api:8000';
    return [
      {
        source: '/api/v1/:path*',
        destination: `${apiUrl}/v1/:path*`,
      },
    ];
  },
};

export default nextConfig;
