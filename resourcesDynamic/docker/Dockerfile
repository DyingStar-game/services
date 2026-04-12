# syntax=docker/dockerfile:1

# ============================================
# Stage 1: Base image with pnpm
# ============================================
FROM node:22.20.0-slim AS base

# Enable corepack for pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

# ============================================
# Stage 2: Install dependencies
# ============================================
FROM base AS deps

# Copy package files
COPY package.json pnpm-lock.yaml ./

# Install all dependencies (including dev for building)
RUN pnpm install --frozen-lockfile

# ============================================
# Stage 3: Build the application
# ============================================
FROM base AS builder

WORKDIR /app

# Copy dependencies from deps stage
COPY --from=deps /app/node_modules ./node_modules

# Copy source code
COPY . .

# Build the application
RUN pnpm build

# ============================================
# Stage 4: Production dependencies only
# ============================================
FROM base AS prod-deps

COPY package.json pnpm-lock.yaml ./

# Install all dependencies (drizzle-kit needed for migrations at runtime)
RUN pnpm install --frozen-lockfile

# ============================================
# Stage 5: Production image
# ============================================
FROM node:22.20.0-slim AS production

# Install npm for running scripts
RUN corepack enable && corepack prepare pnpm@latest --activate

# Create non-root user for security
RUN groupadd --gid 1001 nodejs && \
    useradd --uid 1001 --gid nodejs --shell /bin/bash --create-home nodejs

WORKDIR /app

# Copy production dependencies
COPY --from=prod-deps --chown=nodejs:nodejs /app/node_modules ./node_modules

# Copy built application
COPY --from=builder --chown=nodejs:nodejs /app/dist ./dist
COPY --from=builder --chown=nodejs:nodejs /app/package.json ./

# Copy drizzle migrations (needed for db:migrate at runtime)
COPY --from=builder --chown=nodejs:nodejs /app/drizzle ./drizzle
COPY --from=builder --chown=nodejs:nodejs /app/drizzle.config.ts ./

# Copy data files if needed
COPY --from=builder --chown=nodejs:nodejs /app/data ./data

# Copy entrypoint script
COPY --chown=nodejs:nodejs docker/entrypoint.sh ./entrypoint.sh
RUN chmod +x ./entrypoint.sh

# Switch to non-root user
USER nodejs

# Expose ports (WebSocket and API)
EXPOSE 9200 3001

# Health check
# HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
#     CMD node -e "fetch('http://localhost:3001/health').then(r => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"

# Start the application with entrypoint script
ENTRYPOINT ["./entrypoint.sh"]
