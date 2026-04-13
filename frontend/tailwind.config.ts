import type { Config } from 'tailwindcss';

const config: Config = {
  content: ['./src/**/*.{js,ts,jsx,tsx,mdx}'],
  theme: {
    extend: {
      colors: {
        bg: '#0d0f14',
        surface: '#12151d',
        'surface-elevated': '#1a1e2b',
        border: '#232736',
        'text-primary': '#e2e4ed',
        'text-secondary': '#8b90a2',
        'text-muted': '#575c6e',
        accent: '#3b82f6',
        'mag-green': '#22c55e',
        'mag-yellow': '#eab308',
        'mag-orange': '#f97316',
        'mag-red': '#ef4444',
      },
      fontFamily: {
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'monospace'],
      },
    },
  },
  plugins: [],
};

export default config;
