# ── Stage 1: install dependencies ──────────────────────────────────────────────
FROM node:20-alpine AS deps
WORKDIR /app

COPY package.json package-lock.json* ./
RUN npm ci

# ── Stage 2: build ──────────────────────────────────────────────────────────────
FROM node:20-alpine AS builder
WORKDIR /app

COPY --from=deps /app/node_modules ./node_modules
COPY . .

# Build-time env vars (NEXT_PUBLIC_* get embedded in the JS bundle).
# Override via docker-compose environment or --build-arg.
ARG NEXT_PUBLIC_WS_URL=ws://localhost:9093
ENV NEXT_PUBLIC_WS_URL=$NEXT_PUBLIC_WS_URL

ARG NEXT_PUBLIC_MAP_STYLE=https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json
ENV NEXT_PUBLIC_MAP_STYLE=$NEXT_PUBLIC_MAP_STYLE

ENV NEXT_TELEMETRY_DISABLED=1
RUN npm run build

# ── Stage 3: runtime ────────────────────────────────────────────────────────────
FROM node:20-alpine AS runner
WORKDIR /app

RUN addgroup --system --gid 1001 nodejs \
 && adduser  --system --uid 1001 nextjs

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV PORT=3000
ENV HOSTNAME=0.0.0.0

# standalone output + static assets
COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./
COPY --from=builder --chown=nextjs:nodejs /app/.next/static     ./.next/static
COPY --from=builder --chown=nextjs:nodejs /app/public           ./public

USER nextjs
EXPOSE 3000

CMD ["node", "server.js"]
